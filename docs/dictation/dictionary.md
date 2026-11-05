# Dictation dictionary

> **What this is.** The design of the personal dictionary that dictation recognizes: its data model and store, how privacy tiers filter it, who may read and change it on each surface, how it steers recognition and correction, how it learns from the person's edits, the `dictionary` capability the agent uses to ingest terms, automatic extraction from chats, the left sidebar **Dictionary** page and the terminal commands.
> **Status:** Current: a plan; nothing in it is implemented yet
> **Audience:** developers who build PR-13, PR-14, PR-16, PR-20, PR-22, PR-23, PR-24 and PR-26; reviewers of those pull requests; the owner, who settles O-3 to O-8, O-17, O-21, O-23 and O-31 to O-33.

Each account that runs Biorouter has one dictionary: a file of terms such as UCSF or EPAS1 that dictation should spell the person's way. The daemon, the CLI and the agent's `dictionary` capability read and write that one file under one lock. Dictation feeds the eligible terms to the speech model as a token prompt and a decoder bonus, then corrects the transcript with guarded exact matching. Because Whisper can copy its prompt into the transcript when it hears no speech, a term learned in a private chat must never shape dictation aimed at a public model, so every channel reads one tier filtered set. This document specifies each part to the level an engineer can build from, with the acceptance check for each. The [implementation plan](implementation-plan.md) holds the decision register this document elaborates; where they differ, the plan wins.

## How to read this document

Load bearing claims carry a label:

| Label | Meaning |
|---|---|
| [M] | Measured on 2026-11-04 during this design, on real binaries, browsers or audio. Accuracy numbers here are design measurements on benchmark audio, not claims about the shipped product (DD-95) |
| [C] | Read in Biorouter's code at `main` (`52917f33e`, three commits after the v1.94.1 tag; the workspace version is 1.94.1); a line number is against `main` |
| [R] | Reported by a third party's source, documentation or issue tracker, not measured here |
| [I] | Inferred; the text names the measurement that would settle it when it matters |

Identifiers:

| Scheme | Meaning | Index |
|---|---|---|
| R1 to R8 | The owner's requirements | [Implementation plan](implementation-plan.md), "The owner's requirements" |
| S1 to S20 | The canonical surfaces | [Surfaces](surfaces.md) |
| DD-1 onward | Design decisions; this document elaborates DD-33 to DD-59, DD-67, DD-68, DD-71, DD-75 and DD-77 | [Implementation plan](implementation-plan.md), "Decision register" |
| PR-01 to PR-33 | Pull requests in delivery order | [Implementation plan](implementation-plan.md), "Delivery plan" |
| O-1 onward | Decisions only the owner can make; the text follows each recommended default | [Implementation plan](implementation-plan.md), "Owner decisions" |
| M-1 onward | Measurements still to take | [Testing and release](testing-and-release.md) |
| MSG-… | Exact text a person sees | [Surfaces](surfaces.md), "Message catalog" |
| P-1 to P-15 | Privacy invariants, each held by a named test | [Privacy invariants](#privacy-invariants) |

Code that exists at `main` is cited as a file plus a symbol. Files this plan creates are written as code spans, never as links. Rust and TypeScript blocks are sketches of the shape to build, not finished code.

## What the dictionary is for

| Req | The owner's words, condensed | How this document meets it |
|---|---|---|
| R3 | A personal dictionary of words (UCSF, EPAS1) that dictation recognizes. It grows with the person's workflow, and they can add to it as they go. | The store ([The data model](#the-data-model), [The store](#the-store)); recognition with the dictionary ([How dictation uses the dictionary](#how-dictation-uses-the-dictionary), [Correction after decoding](#correction-after-decoding)); nine ways to add: the sidebar page, **Add to Biorouter dictionary…** in the context menu, **Add to dictionary** on a tool row, `/dictionary add`, `biorouter dictionary add`, import, learning from the person's corrections, the agent's capability and the chat extractor |
| R4 | A component in the left sidebar where people add, maintain and delete entries. | A **Dictionary** row in the sidebar's Components group at `/dictionary`, the same page on the desktop and in every `serve` browser ([The left sidebar Dictionary page](#the-left-sidebar-dictionary-page)) |
| R5 | A plugin or MCP extension that ingests keywords from conversations into the dictionary, "kind of like the same idea as putting things into memory". | The `dictionary` capability with three tools, plus the chat extractor ([The `dictionary` capability](#the-dictionary-capability), [Automatic extraction from chats](#automatic-extraction-from-chats)) |

The default reading of R5:

1. **Plugin or MCP extension.** The capability is a platform extension in the core crate. The agent reaches it through the same tool dispatch as any MCP extension, as `dictionary__add_terms`, `dictionary__list_terms` and `dictionary__remove_added_terms`. It is a platform extension rather than a bundled MCP server because it must read the caller's capability from the dispatch and the calling session's messages, and because app manifests already refuse a platform name they do not list (DD-51).
2. **From conversations.** The agent may take terms from anywhere in the conversation it sees, not only from text the person typed (O-31, DD-55). The narrower rule (only text the person typed in that chat) stays available as a provenance check if the owner prefers it.
3. **Like memory.** Memory has two stores. Its project store is written at once, but its instructions tell the model to act when the person asks and otherwise "offer to remember it and wait for agreement before writing" [C `crates/biorouter-mcp/src/memory/mod.rs` `base_instructions`]. Its machine wide store needs the person's approval for every read, write and delete [C `crates/biorouter/src/security/global_memory.rs` `GlobalMemoryInspector`]. The dictionary is machine wide, so by default an agent's term lands as a suggestion the person accepts (`suggest`, O-3). Memory's own protocol (`on_request`: a term the person asked for, or agreed to when the agent offered it, lands at once) and immediate writes with **Keep** and **Undo** (`add`) are the person's settings on the Dictionary page, not new code (DD-53). The recommended default departs from memory twice. The capability is off until the person ticks **Suggest dictionary words from my chats** at the first model install, while memory is on in every chat by default [C `crates/biorouter/src/config/extensions.rs` `inject_builtin_extensions`, `builtin_entry` with `enabled: true`]. Agent terms then wait for review even when the person asked for them, while memory writes its project store at once when asked. So `suggest` is the narrower reading of R5, and `on_request` is the literal one (O-3).
4. **Ingestion that needs no agent turn.** The chat extractor finds term shaped tokens in the person's own messages and proposes them as suggestions. It ships switched on only if it passes the precision gate M-21 (DD-57).

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

The store file:

```json
{
  "format": "biorouter.dictionary.store",
  "version": 1,
  "settings": { "learn_from_chats": false, "agent_writes": "suggest" },
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
      "provenance": null
    }
  ],
  "tombstones": [{ "fold_key": "phd2", "tier": "private", "at": "2026-11-03T12:00:00Z" }]
}
```

`learn_from_chats` in a new store takes the compiled default `LEARN_FROM_CHATS_DEFAULT`, which PR-24 sets from the M-21 result ([The M-21 gate](#the-m-21-gate)). `agent_writes` starts at `suggest` (O-3).

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
pub enum AgentWrites { Suggest, OnRequest, Add } // ordered: raising means a later variant

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
}

pub struct Misrecognition { pub text: String, pub count: u32, pub last_seen: DateTime<Utc> }
pub struct Tombstone { pub fold_key: String, pub tier: EntryTier, pub at: DateTime<Utc> }
```

### Validation

`crates/biorouter-dictation/src/store/validate.rs` is the one validator. The routes, the capability, the extractor, the learner, the CLI and import all call it. A term (and each spoken form, misrecognition and cue) is refused when:

| Code | Rule |
|---|---|
| `empty` or `too_long` | fewer than 1 or more than 64 characters after NFC and trim (cues: more than 32) |
| `too_many_words` | more than 6 whitespace separated words |
| `line_break` | contains CR or LF |
| `control_character` | contains any Unicode control character |
| `angle_bracket` | contains `<` or `>` |
| `url` | contains `://` or starts with `www.` |
| `email` | has the shape `local@domain.tld` |
| `path` | starts with `/`, `~`, `./`, `../` or a drive letter, or contains two or more `/` or `\` separators |
| `digits_only` | only digits and separators |
| `long_hex` | a run of 16 or more hexadecimal digits |
| `secret_shaped` | a known key prefix (`sk-`, `AKIA`, `ghp_`, `xox`) or a base64 shaped run of 32 or more characters |
| `common_word_misrecognition` | a misrecognition that is a single common English word, on an `agent`, `auto` or `correction` row. The learner applies the same rule to every row it changes ([Effects on the dictionary](#effects-on-the-dictionary)) |

The leaf crate cannot call the core's secret scanner, so every caller that can (the routes, the capability, the extractor and the CLI) also refuses a term for which `biorouter::guardrails::secret_output::redact_text` returns `Some` [C `crates/biorouter/src/guardrails/secret_output.rs` `redact_text`].

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

`usage.json` sits beside the store with two maps of the same shape: `{"all": {"<id>": {"uses": 14, "last_used_at": "2026-11-04T17:02:11Z", "corrected_to": 2}}, "public": {"<id>": {…}}}`. It is a separate file so a usage bump never changes an entry's `rev` (a spurious `412` on a person's open edit) and never rewrites the entries file per utterance (DD-33). `uses` and `last_used_at` move when a dictation's final text contains the row's written form as a whole word. `corrected_to` moves when the person corrects dictated text to the row ([Effects on the dictionary](#effects-on-the-dictionary)). Every bump goes to `all`. A dictation decoded with the public rows table, and a correction stamped public, also bump `public`. Ranking for a public scope request reads only `public`, so private chats' dictation never changes which public terms a public prompt carries (oracle rule 11). The usage fields a caller with private standing sees come from `all`. The service batches bumps in memory and flushes at most every 30 s and at the worker's idle unload, with the extractor's lock budget; a lost bump is acceptable.

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

At about 300 B an entry, 5,000 rows are about 1.5 MB per tier [I].

### Validation and cap texts

These sentences are in the message catalog in [Surfaces](surfaces.md) under the ids below, and `components/dictionary/copy.ts` and `crates/biorouter-cli/src/dictation/messages.rs` read them from there:

| Code | Id | Text |
|---|---|---|
| `too_long` | MSG-TERM-TOO-LONG | Use a term of at most 64 characters. |
| `too_many_words` | MSG-TERM-WORDS | Use a term of at most six words. |
| `line_break`, `control_character`, `angle_bracket` | MSG-TERM-PLAIN | Use one line of plain text. |
| `url`, `email`, `path` | MSG-TERM-ADDRESS | A dictionary term cannot be a web address, an email address or a file path. |
| `digits_only`, `long_hex`, `secret_shaped` | MSG-TERM-IDENTIFIER | This looks like an identifier or a key, so it was not added. |
| `cap` | MSG-DICTIONARY-CAP | Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one. |
| `stale` | MSG-DICTIONARY-STALE | This term changed since you opened it. Reload it and try again. |

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

The core resolves the folder with `Paths::in_config_dir("dictionary")` [C `crates/biorouter/src/config/paths.rs`] and passes it to the leaf crate, which takes every path as an argument. The folder holds `dictionary.json`, `usage.json` and `.lock`. `crates/biorouter/tests/path_resolver_agreement.rs` gains a row for it. The store holds no absolute paths, so the Windows legacy folder rewrite needs no row. Two machines, and a Windows desktop and its WSL account, have two dictionaries; export and import move terms between them ([Syncing two hosts](#syncing-two-hosts)).

### Writing and locking

Memory's pattern [C `crates/biorouter-mcp/src/memory/mod.rs` `StoreLock`, `lock_store`] (DD-33):

1. Take an exclusive `fs2` lock on `.lock` (`fs2` 0.4.3 is in `Cargo.lock` [C]).
2. Read and parse both files fresh under the lock.
3. Apply the change; bump `rev` and `updated_at` on every changed row.
4. Write `dictionary.json.tmp`, `fsync` it, rename it over `dictionary.json`, `fsync` the folder on Unix. On Windows a rename refused by an open handle is retried up to 10 times over 200 ms [I; size the retry against antivirus scanning once on a scanned profile].
5. Release the lock.

The folder is created 0700 and the files 0600 on Unix, because private rows may sit there.

Two budgets: the daemon's and the CLI's ordinary writes wait for the lock. The extractor and usage flushes use `try_write_within(50 ms)`, which retries `try_lock_exclusive` every 5 ms and drops the write when the budget ends.

### Reading and caching

Readers parse without the lock. The daemon and an interactive CLI cache the parsed view keyed by the (mtime, length) of both files and reparse on any change, so a term added in one process is listed by another at once. Each reparse increments an in process generation number. The generation is never exposed: no route reports a store wide revision (DD-40).

### When the file cannot be read

A parse failure, an unknown `format`, or a `version` other than 1 fails closed:

- every listing is empty, every dictation runs with an empty bias set, and every write and observation is refused with MSG-DICTIONARY-FILE: "The dictionary file at `<path>` could not be read, so nothing was changed. Fix or remove it, then try again.";
- the file is never rewritten from a failed parse, so the person can repair it by hand;
- `biorouter doctor` and `biorouter dictation status --verbose` name the file.

### Store API

```rust
// crates/biorouter-dictation/src/store/mod.rs
pub struct DictionaryStore { dir: PathBuf }

impl DictionaryStore {
    pub fn open(dir: impl Into<PathBuf>) -> Self;
    /// Cached by the (mtime, len) of dictionary.json and usage.json.
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
    Io(std::io::Error),
}
```

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

`max` orders Private above Public. An edit whose stamp rises above the row's tier is written into the private twin (created from the public row if absent), and the public row stays unchanged; an edit never moves a row between tiers, because that would be a privatize, which a keyless daemon refuses (DD-38) and which would make the public row vanish (DD-35). On a keyless daemon no stamp is ever above the caller's standing, so a caller without private standing never writes into the private tier and never learns what it holds (oracle rule 1, P-14).

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
    table: BonusTable,                // PublicRows | AllRows: which decoder bonus table to use
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
/// crates/biorouter-cli/src/dictation/consent.rs (interactive terminal, BIOROUTER_TERMINAL unset).
pub struct Standing { private: bool }
impl Standing {
    pub const fn public() -> Self { Self { private: false } }
    #[doc(hidden)]
    pub const fn from_checked(private: bool) -> Self { Self { private } }
    pub const fn is_private(&self) -> bool { self.private }
}

pub fn bias_set(view: &StoreView, target_private: bool, standing: Standing, enforced: bool) -> EligibleEntries {
    // Private reach only for a private capable target AND a caller with private standing.
    let reach = if !enforced || (target_private && standing.is_private()) {
        ProviderTier::Private
    } else {
        ProviderTier::Public
    };
    let rows: Vec<RecognitionEntry> = view
        .entries()
        .filter(|e| e.status == EntryStatus::Active && e.bias != BiasPolicy::Never)
        .filter(|e| privacy::visible_to(reach, classification_of(e.tier)))
        .map(RecognitionEntry::from)
        .collect();
    let table = if reach.is_private() { BonusTable::AllRows } else { BonusTable::PublicRows };
    EligibleEntries::new_filtered(merge_twins(rows), table)
}
```

The predicate is the plan's: `status = active ∧ bias ≠ never ∧ (¬enforced ∨ tier = public ∨ (target_private ∧ standing.private))`, compared through `privacy::visible_to` [C `crates/biorouter/src/privacy/mod.rs:417`]. The census in `crates/biorouter/tests/privacy_guard_wiring.rs` gains `bias.rs` as a `visible_to` site and a row for `bias_set` itself ([`HttpCaller` methods and the census](#httpcaller-methods-and-the-census)).

The Tier 1 accuracy test builds its dictionary through `bias_set` like every other caller, so it lives in the core crate as `crates/biorouter/tests/dictation_accuracy.rs`. A test in the leaf crate's own `tests/` folder could reach `EligibleEntries` only through `new_filtered`, which the repository walk test forbids.

The transcript's `bias_scope` is `private` only when the switch is on, the target is private capable and the caller has private standing; it is `public` for every other caller, including any caller without private standing (DD-29). With the switch off the set holds every row and `bias_scope` reports `public`, so the private draft flag stays silent.

Who decides `target_private` (DD-30, never the client):

| Request | `target_private` |
|---|---|
| `POST /dictation/streams` with `session_id` | `session_reach` first; then true only when `chat_reach` answers Private over `target_tier` [C `crates/biorouter-server/src/routes/session_reach.rs` `target_tier`] and the live agent's bound provider (`AppState::peek_agent`). `Unknown` counts as false here. A refusal silently degrades to false with a byte identical response |
| `target_kind: "new_chat"` (Home, a new tab) | the tier the new chat's bind would produce (`new_chat_bind_tier`: the provider built from the capability configuration keys, its instance `tier()`, a lead and worker pair reduced with `ProviderTier::least`, Public on any failure), never a declared tier; the per tab pick (`new_chat_provider`) counts only from a proven caller |
| `target_kind: "crew"` or `"external"` | false, unless the body carries `include_private_terms: true` from a caller with the proof and private standing, which the renderer sends only after the person confirms MSG-PRIVATE-TERMS for that recording (O-8) |
| TUI and REPL | the session's tier, in process |
| Shared daemon prompt, `session attach` | the chat's stored tier, read offline, and only at an interactive terminal |
| `transcribe` or `dictate` with `--session <id>` | DD-77: the chat is private AND `prompt_can_run()` AND `caller_capability()` is Private AND `BIOROUTER_TERMINAL` is unset |
| `dictate` to stdout or `dictate --copy`, no `--session` | DD-77: false unless the person passes `--private-terms`, admitted only when `prompt_can_run()` is true, `BIOROUTER_TERMINAL` is unset and `caller_capability()` is Private, after MSG-PRIVATE-TERMS and a `y` (O-8) |

Public by default, with private terms only for one recording the person confirms after MSG-PRIVATE-TERMS, and never for an agent (O-8): Crew's composers (O-21), **Dictate to clipboard** and app preview inserts (`external`), and `dictate` to stdout or `--copy`. Public only, whatever the caller: the Jupyter handler, `transcribe` without an admitted `--session`, and ACP, because no proof of a person reaches their transcription.

Why one filtered set and not "every term, because audio stays on the machine": the text lands in a composer whose content goes to that chat's model. Without voice detection, 4 of the 64 nonspeech decodes emitted dictionary terms outright (`PHD2, PHD2, PHD2`) [M]. With a text prompt and the decoder bonus, time reversed speech produced real symbols (`PHD2A, BRCA1`, voice detection off), and with the detector on the reversed speech still passed it and one dictionary term came through [M]; the token prompt with the bonus is unmeasured on nonspeech (M-9). A correction writes a term as surely as an echo does [I].

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
9. Decoder bonus tables for both scopes (public rows, all rows) are rebuilt at every dictionary revision, before the next request, so a request's latency never depends on which tier changed.
10. No response reports a count of eligible or withheld entries.
11. Ranking for a public scope request reads only the `public` usage map ([Usage](#usage)), so its prompt and its fuzzy set are identical whatever private scope dictations and private corrections have happened.

### The private draft flag

Every transcript reports `bias_scope`. The renderer stores it on the dictated span (`DictatedSpan.biasScope`, [Desktop and `serve` ledger](#desktop-and-serve-ledger)). When a draft holds text dictated with `private` scope and the person sends it to a chat bound to a public model, the composer asks MSG-PRIVATE-DRAFT: "This message was dictated with your private dictionary terms turned on, so it may contain them, and `<model>` is a public model. Send it anyway?" with **Send anyway** and **Cancel**. "A public model" is judged by the daemon's instance tier for that chat, never by a declared tier. Switching that chat to a public model asks MSG-PRIVATE-DRAFT-SWITCH first. Either proceeds on the person's say so (DD-71). No agent path ever sees or answers the flag. The composer behaviour is specified in [Surfaces](surfaces.md).

### What is and is not claimed

The tier rules stop a cooperative agent or a person's slip from putting private terms into a public chat's dictation. They do not stop a determined actor with a shell, who can read `dictionary.json` directly (DR-17 left the filesystem open), unset `BIOROUTER_TERMINAL`, or relocate the managed policy file with `BIOROUTER_PATH_ROOT`. On a keyless daemon, any holder of the daemon secret, including an agent shell that recovers it, can install a pinned model and make the dictionary writes a `serve` browser may make (add, accept, dismiss, edit, delete and import on rows it can list; on a host whose operator's provider is private, that includes private rows). An agent holding a Biorouter Copilot grant on the person's web browser can press the microphone in a `serve` tab or the Jupyter AI chat, where the site's microphone grant records with no new prompt (DD-88, SD-16). Every control in this document is accident prevention for cooperative actors. Privacy tiers here, as everywhere in Biorouter, are a safety boundary against mistakes by cooperative actors, not a security boundary against a determined adversary.

Dictionary rows have no affiliation axis (O-32). A private term may bias dictation aimed at any private model, of any institution, although HIPAA compliance does not transfer between institutions (DR-26, [institutional affiliation](../security/institutional-affiliation.md)). The plan accepts this because terms are screened for PII and secrets on every write path and are vocabulary, not records. If the owner decides otherwise, private rows carry the writer's affiliation (`CallCapability::affiliation()` [C `crates/biorouter/src/privacy/capability.rs`], `McpMeta.capability_affiliation` [C `crates/biorouter/src/agents/mcp_client.rs`]) and `bias_set` filters them by the rule `CallCapability::cross_affiliation` applies to extensions [C `crates/biorouter/src/privacy/capability.rs`].

## Authority on each surface

### Who may do what

Private standing means the user action proof, a private capability the request states in `X-Caller-Provider` (which a `serve` page on a private host sends with every call, SD-12), or a person at an interactive terminal. A `serve` daemon's operator tier (`served_operator`), which SD-10 lets listings read, lists private rows to the served page but is never standing by itself: it never biases dictation or stamps a write (DD-30).

| Surface or caller | Read public rows | Read private rows | Write | Tier change | Private bias |
|---|---|---|---|---|---|
| Desktop app on its own keyed daemon (S1 to S4) | yes | yes; every call sends `userActionHeaders()` | with the proof | with the proof, after MSG-PUBLICIZE-WARN before publicize | private target and the proof |
| `serve` browser, keyless daemon (S5, S6, S8, S16) | yes | with private standing only | admitted for rows the caller can list (O-4); stamped with the caller's standing; a private add without private standing refused with MSG-DICTIONARY-HOST-PRIVATE | refused; disabled before the click with MSG-DICTIONARY-HOST-TIER | private target and private standing |
| External backend, with that server's User Action Key (S17) | yes | yes | as the desktop | as the desktop | as the desktop |
| External backend, keyed server, no key (S17, O-17 recommended) | yes | no | refused; disabled before the click with MSG-REMOTE-NO-KEY | refused | never |
| External backend, keyless server (S17) | as `serve` | as `serve` | as `serve` | as `serve` | as `serve` |
| TUI and classic REPL (S9, S19) | yes | yes | yes, in process (`/dictionary add`, `/dictionary review`) | not from the session; `biorouter dictionary publicize` at a terminal | when the session is private capable |
| `biorouter dictionary` at an interactive terminal | yes | yes (DD-37) | yes | after a typed confirmation (O-5) | n/a |
| Noninteractive CLI (pipe, cron, `run`, `session send`) | yes | no | refused (exit 2), except `import` ([Terminal only rules](#terminal-only-rules)) | refused | public only |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | yes | no | refused with MSG-DICTIONARY-AGENT (exit 2) | refused | public only, whatever the flags (DD-77) |
| The agent's tools (`dictionary__*`) | its tier scope | only for a private capability caller | `add_terms` per `agent_writes`; `remove_added_terms` on its own unreviewed rows | never | n/a: an agent never transcribes |
| ACP stdio (S12) | through the tools | as the tools | suggestions only, in every mode | never | public only |
| ACP `--ws` | no capability | no | no | no | public only |
| Jupyter AI (S13) | through the persona's ACP tools; a JupyterLab terminal | at a JupyterLab terminal | suggestions only through the persona | at a JupyterLab terminal | public only |
| Crew (S14) | the member's own sidebar | the member's own sidebar | the member's own sidebar; Crew scoped chats cannot call `dictionary__*` | the member's own sidebar | public by default; private terms for one recording after MSG-PRIVATE-TERMS (O-8, O-21) |
| Agent Drafter apps (S15) | no | no | no; manifests refuse the capability | no | public by default; the desktop preview's `external` target takes private terms for one recording after MSG-PRIVATE-TERMS (O-8) |
| **Dictate to clipboard** (S18) | n/a | n/a | n/a | n/a | public by default; private terms for one recording after MSG-PRIVATE-TERMS, never for an agent (O-8) |

### Routes

`crates/biorouter-server/src/routes/dictionary.rs` (PR-16), merged behind `check_token`, utoipa annotated, then `just generate-openapi && cd ui/desktop && npm run generate-api`. Every `{id}` route sits behind `gate_dictionary_entry`, a `route_layer`.

| Route | Request | Answer | Keyed daemon | Keyless daemon (`serve`) |
|---|---|---|---|---|
| `GET /dictionary/entries?status=&q=&cursor=` | `status` `active`, `suggested` or `all` (default); `q` a case folded substring of written, spoken forms and note; `cursor` opaque | 200 `{entries: [EntryView], next_cursor}`, at most 500 rows a page, sorted by `updated_at` descending then `id`; `ETag`; 304 on `If-None-Match` | listing by standing | listing by standing |
| `POST /dictionary/entries` | `{written, spoken_forms?, kind?, note?, private?, match?, bias?, context_cues?}` | 201 `{entry, warning?}`; 200 `{entry, created: false}` when the row already exists in the stamp tier | proof | admitted; stamped with the caller's standing; `private: true` without private standing refused with MSG-DICTIONARY-HOST-PRIVATE before any row is read |
| `PATCH /dictionary/entries/{id}` with `If-Match: <rev>` | any of `{written, spoken_forms, kind, note, match, bias, context_cues}` | 200 `{entry}`; 412 `{code: "stale", rev}` | proof | admitted if listable; stamped with the caller's standing |
| `DELETE /dictionary/entries/{id}` | none | 204; a tombstone per (`fold_key`, `tier`) | proof | admitted if listable |
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
| `POST /dictionary/activation` | `{suggest_from_chats}` | 200 as `GET /dictionary/settings`; calls `activation.rs` (DD-56) | proof | `false` admitted; `true` refused with MSG-DICTIONARY-HOST-SETTINGS |

There is no `POST /dictionary/clear`. A full wipe is `biorouter dictionary clear` at a terminal with a typed phrase, and the page's **Delete all terms…** deletes the listed rows through `POST /dictionary/entries/delete` (DD-38). Managed `dictionary: false` refuses writes and observations with MSG-POLICY; export and delete stay allowed.

`EntryView` is the stored entry minus `seen`, plus the usage fields only for a caller with private standing:

```json
{
  "id": "dct_6f0b8a52-6a1e-4f7e-9d0b-2a3c1d4e5f60", "rev": 3, "written": "EPAS1", "tier": "public",
  "status": "active", "source": "user", "reviewed": true, "spoken_forms": ["epas one"],
  "misrecognitions": [{ "text": "a pass one", "count": 2 }], "kind": "gene_symbol", "match": "exact",
  "bias": "auto", "context_cues": [], "note": null, "reason": null, "provenance": null,
  "created_at": "2026-11-01T09:00:00Z", "updated_at": "2026-11-04T17:02:11Z",
  "uses": 14, "last_used_at": "2026-11-04T17:02:11Z", "corrected_to": 2, "suggested_in": "20261104_12"
}
```

`authority` in the settings answer tells the page, before any click, what this caller may do, so no surface offers a doomed control. The answer below is a `serve` browser on a host whose operator's provider is public:

```json
{
  "suggest_from_chats": true,
  "learn_from_chats": true,
  "agent_writes": "suggest",
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

Census changes in `crates/biorouter/tests/privacy_guard_wiring.rs` (`routes/session_reach.rs` is one of its three `COMPLETE_MODULES`, so every new `pub fn` there needs a row):

- five new `Guard` rows, `lists_dictionary_entry`, `dictation_standing`, `authorizes_dictionary_write`, `dictionary_write_stamp` and `gate_dictionary_entry`, each `defined_in: SESSION_REACH` and `Status::Wired`;
- two rows written by hand, because the census discovers new predicates only in its `COMPLETE_MODULES` and says to add a row by hand for one outside them [C module doc]: `bias_set` (`defined_in: crates/biorouter/src/dictation/bias.rs`, `Status::Wired`, with its call sites in `routes/dictation.rs` and the CLI's dictation module) and `chat_reach` (`defined_in: crates/biorouter/src/dictation/target.rs`, `Status::Wired`, with its sites in `routes/dictation.rs`, `routes/dictionary.rs` and `extractor.rs`);
- new sites for `session_reach` and `http_caller` in `routes/dictionary.rs` and `routes/dictation.rs`, and for `visible_to` in `bias.rs`;
- a new site for `target_tier` in `routes/dictation.rs`, which moves its row from `Status::WiredThrough("session_reach")` to `Status::Wired`, because at `main` it has no caller outside `session_reach.rs` [C].

The table of `every_gated_route_resolves_the_tier_before_it_touches_the_session`, a test in `crates/biorouter-server/src/routes/session_reach.rs` [C `:1782`] rather than in the census file, gains a row for the observations handler. A repository walk test, `private_standing_is_built_in_two_files`, pins `Standing::from_checked` to `HttpCaller::dictation_standing` and `crates/biorouter-cli/src/dictation/consent.rs`. No new `floor`, `raise_privacy`, `.call_tool(` or `CallCapability::sample(` site.

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
    decoder bonus: whole bias set                    engine/booster.rs
 -> greedy decode, timestamps on                     engine/mod.rs
 -> whole output filter, echo and collapse checks, at most one retry   recognize/echo.rs
 -> correction: aliases, exact keys, guarded fuzzy  recognize/correct.rs
 -> {text, raw_text, changes[], bias_scope, ...}
```

The service sends `set_bias` for both bonus tables (public rows, all rows) whenever the store generation changes, and waits for `bias_ready` before the next `transcribe`. A `transcribe` frame carries the ranked written forms for the prompt and names the table; the worker encodes the prompt, decodes and returns the segments and the exact prompt string it sent, which the service keeps in memory for the echo check and never logs.

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
      - 2.0 * saturated                                uses >= 5 and corrected_to == 0
ties: updated_at descending, then id
```

`uses`, `days_since_last_used`, `corrected_to` and `saturated` read the `public` usage map when the request uses the public rows table, and the `all` map otherwise (oracle rule 11). The weights are a starting point [I], tuned once on the lab set. `in_active_kb` is 0 for every entry in v1, because no field yet records which knowledge base a term came from. Two consumers take a slice of the ranking: the prompt (top 30) and the fuzzy set (the `match = fuzzy` entries among the top 30). The decoder bonus, the alias automaton and the exact keys take the whole bias set unranked.

### Token prompt

`recognize/prompt.rs` picks; `engine/bpe.rs` encodes (DD-45):

1. Eligible: in the bias set, and not `collides` unless the entry is in `context` or pinned.
2. Take the top 30 by score. When fewer than 30 entries match the context, the remaining places go to the next entries by score, where pinned and recent ones rank; the prompt is never only the terms predicted for this utterance, because an oracle narrow prompt cost 18 points on real speech by deleting the start of the utterance [M].
3. Written forms only, joined with `", "`, no lead in, no tail, no leading space, ascending score so the best entry is last. Spoken forms and misrecognitions are never prompt content.
4. Encode with canonical byte level BPE whose ranks are the loaded model's own token ids, and pass the ids as `prompt_tokens` with `initial_prompt` unset. Cut entries from the head until the prompt is at most 150 tokens. Whisper's window is 223 tokens and drops its head silently [R].
5. No draft text in the prompt in v1.

The encoder: ranks equal the model's token ids (turbo's vocabulary differs from `multilingual.tiktoken` only at id 188; `.en` models use the GPT-2 vocabulary) [M]; the GPT-2 pre split regex through `fancy-regex` (in `Cargo.lock` [C]); `tiktoken-rs` 0.6.0 (already a core dependency [C]) if its `CoreBPE` takes an arbitrary rank map, otherwise a byte level encoder of about 100 lines. No tokenizer text file ships. Golden splits: ` EPAS1` to ` EP|AS|1` and ` Biorouter` to ` B|ior|outer` [M].

Why ids and not text: whisper.cpp tokenizes `initial_prompt` text by greedy longest match, so the model copies a segmentation it was never trained on (` EPA|S|1`). On synthetic speech, the same list as canonical ids raised turbo from 80 to 93 % pooled strict recall and small.en from 85 to 90 % [M]. On real speech a text prompt was the largest single gain measured, +24 to +33 points on gene symbols [M]; ids on real speech are part of M-9.

Budget: 30 terms cost 115 to 118 tokens [M], so the 150 token cap rarely binds. A 54 term prompt (209 to 217 tokens) cost 2 to 10 points against 14 terms on synthetic speech [M].

### Decoder bonus

`engine/booster.rs`, a `logits_filter_callback` over the whole bias set, greedy decoding only (DD-46):

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

The callback is reentrant and runs inside `std::panic::catch_unwind`. Tables are built per (model hash, store generation, table) with an LRU of 4, and both tables are rebuilt at each generation (oracle rule 9). Measured on synthetic speech: 33 to 52 ms to build, 0.57 ms per utterance; 72 % strict recall at 1,000 terms against 49 to 52 % for the same terms as a prompt; 0 dictionary insertions on 75 probe clips [M].

### Decoding parameters

Every dictation request (DD-47); the full table with reasons is in [Engine and models](engine-and-models.md):

| Parameter | Value |
|---|---|
| strategy | `WHISPER_SAMPLING_GREEDY`, `best_of = 5` |
| `language` | `en` for `.en` models; the configured language for turbo, never `auto` unless set |
| `translate`, `no_context` | false, true |
| `no_timestamps` | false; timestamps stripped from text. Timestamps off collapsed small.en on 12 of 49 real utterances, 2 of 49 with them on [M] |
| `single_segment` | false |
| `temperature`, `temperature_inc` | 0, 0.2 until M-9 |
| `entropy_thold`, `logprob_thold`, `no_speech_thold` | 2.4, -1.0, 0.6 |
| `suppress_blank`, `suppress_nst` | true, false (`suppress_nst` would block `HLA-B*57:01` and `C++`) |
| `prompt_tokens`, `initial_prompt` | set, unset |
| `flash_attn` | on |
| audio per `whisper_full` call | at most 25 s, asserted on the parameter builder |

### Voice detection and nonspeech filters

Silero always on, on every surface (DD-48): threshold 0.50, minimum speech 250 ms, minimum silence 100 ms, `speech_pad_ms = 30`, and 0.3 s kept before the first and after the last segment. On 16 real gaps (10 of room tone and 6 where only the other party bleeds in, bracketed tags not counted), turbo's output fell from 15 or 16 of 16 to 1 of 16 with the detector, and small.en's from 2 to 8 of 16 to 0 of 16 [M]. No segments or an all zero buffer: nothing is inserted, and the transcript reports `no_speech`.

A whole output filter in `recognize/echo.rs` runs after trimming punctuation and removes the output only when all of it is one of: a run of bracketed or parenthesized tags, `you`, `Thank you.`, `Thank you for watching!`, `Okay.`, `so`, `and`, `.`, `Grazie`, `Grazie a tutti`, `Sous-titrage Société Radio-Canada`, `Gracias por ver el video`, `Altyazı M.K.`. Parentheses inside real speech are never stripped.

### Echo, collapse and one retry

Checked against the exact prompt string sent, not against the dictionary:

| Check | Rule |
|---|---|
| Whole echo | 90 % or more of the output's unique words are prompt words, and it continues the prompt's shape: a word repeated 3 or more times, a short fragment (at most 30 characters) ending on the separator, or 3 or more consecutive entries in prompt order [R: OpenWhispr's heuristic, `analyzeDictionaryPromptFragment` in `src/utils/dictionaryEchoFilter.js`; whole prompt echo and term looping on Whisper are reported in OpenWhispr issue #1454 and in that file's comments] |
| Trailing recital | a sentence copying 2 or more consecutive entries from the prompt head, or 5 or more ending at its tail [R: OpenWhispr's `stripTrailingDictionaryEcho` in the same file; the recital was reported on Qwen3-ASR-1.7B, a prompt conditioned model, in OpenWhispr issue #2581, with related reports in #1547 and #1889] |
| Collapse or deleted start | fewer than 1.0 output words per second over more than 3 s of detected speech [I] |

The 90 %, 3, 2 and 5 thresholds are OpenWhispr's design constants, not measurements of Whisper, and the trailing recital is not reported on Whisper at all. Biorouter's own evidence for echo is the nonspeech decodes in [The bias set](#the-bias-set); how often these checks fire wrongly on real speech is unmeasured.

On an echo, a collapse or a deleted start: one retry with no prompt, the voice detector and the bonus still on. The retry is adopted only if it is not itself prompt like; otherwise the transcript is `no_speech`. A trailing recital is stripped, not retried. The transcript reports `retried: {reason}` with no text. A retry without the voice detector is never made, because it reopens hallucination [M].

### The M-9 gate and the declared fallback

The token prompt and the decoder bonus together ship as the default only if M-9 shows the combination on real speech at least as good as each alone, with no rise in nonspeech output, probe insertions or collapses (DD-44). Otherwise the declared fallback is the token prompt only, with the bonus off, because the prompt is the only channel with real speech evidence. The choice is a compiled default (`BiasChannels { prompt: true, bonus: <M-9> }`) changed only by a release; correction always runs. M-9 also decides `temperature_inc` (0 against 0.2).

### Long dictation

During a stream, once 8 s or more are buffered the daemon asks the worker for voice segments and queues each closed segment (at most 25 s, split at the longest pause), decoded with `no_context = true` and the same prompt and table; the last segment is decoded on stop (DD-50). Each segment is corrected for the `partial` preview line. At finish, correction runs once more over the joined raw text, so `changes` offsets refer to the final text. M-19 measures boundary word loss on real continuous speech and blocks announcing v1.

**Acceptance.** PR-14: BPE parity with golden ids committed as numbers per vocabulary (only 270 ids hold the same bytes in the GPT-2 and multilingual vocabularies [M], so no id list is shared between them); a 600 token dictionary trims to at most 150 tokens with the top entry last; the bias table test (target tier, entry tier, standing, switch; clipboard, Crew, app and unknown CLI targets public unless `include_private_terms` or `--private-terms` is admitted under O-8); `eligible_entries_are_built_only_by_bias_set`; bonus cases on synthetic logit rows (never at or above end of text, identical from several threads, under 100 µs a call); an agent shell `transcribe --session <private id>` on reversed speech, on a machine configured for a private provider, returns no private term; the `#[ignore]` bonus test reproduces 70 % strict or more at 1,000 terms with small.en; one dispatched Tier 1 run records the dictionary arm. Token count goldens: the 14 term list costs 62 tokens (small.en) and 63 (turbo) under both `whisper_tokenize` and canonical BPE [M]; the 26, 37 and 14 term lists cost 111, 149 and 62 tokens on the `base.en` vocabulary (as small.en) and 119, 160 and 63 on the `tiny` vocabulary (as turbo) under `whisper_tokenize` [M]; PR-14 commits the canonical counts for the two longer lists from its reference encoder, which can be up to 2 % higher [M].

## Correction after decoding

One pass in `crates/biorouter-dictation/src/recognize/correct.rs`, every change recorded as `{start, end, from, to, stage, entry_id}` and listed in the transcript's `changes` (DD-49). `transcribe --format json` schema 1 uses the same names as the HTTP `Transcript`: `changes` with `stage`, `audio_s` and `bias_scope`, so the Jupyter labextension and the renderer read one shape.

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

The inserted text is the entry's written form in its own case; `gene_symbol` and `acronym` kinds never take the spoken token's case. On every real speech run the exact key stage made no false edit; on synthetic speech it turned `PhD two` into PHD2, which the `digits_literal` guard stops, and `spoke` into SPOKE with the single token guard off [M].

### Step 4: guarded fuzzy matching

Only for entries in the fuzzy set (the `match = fuzzy` entries among the top 30), and only when every guard holds:

| Guard | Rule |
|---|---|
| Span | 1 to 3 tokens, no punctuation inside, key length 4 or more |
| Shape | length ratio between 0.67 and 1.5; equal digit strings |
| Similarity | normalized Levenshtein similarity of keys 0.80 or more, or 0.60 or more with Double Metaphone similarity 0.75 or more (`strsim` 0.11.1 is in `Cargo.lock` [C]; Double Metaphone is implemented locally, not added as a dependency) |
| Abstain | the best and the runner up are within 0.04 |
| English | no token of the span is in the word list (with inflections and contractions) unless the entry is in `context` or a `context_cue` occurs |
| Collisions | the span is not a correctly spelled other entry, and not a stop list near collision: UCSD and UCSF, NTRK and NRAS, exon and EGLN1, CRISPR and CRISPRi, clinic and Clenil, continue and codeine, smoke and SPOKE, coding and codeine |
| Prompt pieces | the span is not a truncated piece (a prefix of 4 or more characters) of another entry that was in the prompt sent |
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

### What was measured

Fuzzy matching is what makes coined names work (`ventilin`, `klenl`, `FLET3`) [M], and over a 26 term drug list the guarded corrector made 0 genuine false edits across 7,108 real consultation utterances [M]. It is unsafe for short symbols: over a 37 term gene list, the guarded pass made 45 false edits across 4,651 real sentences (`here's` to HRAS 21 times, `aren't` and `ain't` to ARNT 12 times, `ras` to HRAS, `norms` to NRAS) [M]. Over the same sentences the 14 term list of the synthetic benchmark made 13 false edits (`UCSD` to UCSF 4 times, `exon 1` to EGLN1 3 times, `spikes` to SPOKE, `CRISPR` to CRISPRi) [M]. On real audio, the corrector turned NTRK, heard as `N-TRAC`, into NRAS [M]. The defaults above and the stop list would have blocked every one of those measured false edits [I]; M-9 and the Tier 1 gate check that on real corpora.

### The English word list

`crates/biorouter-dictation/assets/english-words.zst`: a SCOWL derived list (size 60, US and GB spellings, with inflections and contractions), lowercase, zstd compressed, embedded with `include_bytes!`, generated by a script under `scripts/dictation/` from a pinned source with every word the retired name guard (R-31) matches removed, plus the hand list `phd uc ucsd crispr rna dna pdf`. It drives `collides`, the bonus's start piece filter, the single token exact guard, the fuzzy English guard, the wordy alias rule and the learner's filters. A system word list is never used: it lacks `coding`, `here's`, `aren't`, `encodes` and `cells`, and is absent on Windows [M].

**Acceptance.** PR-14: the key table (`E-PAS-1`, `EPAS 1`, `E P A S one` all to `epas1`); positives (`ventilin`, `klenl`, `FLET3`, `ALAS5AP`, `P-R-O-K-R2`, `IGF-1R`, `E-PAS-1`, `pH D2`) and negatives that must stay unchanged (`here's`, `aren't`, `UCSD`, `exon 1`, `N-TRAC`, `clinic`, `continue`, `coding`, `smoke`, `PhD two`, `crisper drawer`, `Oh mop it up`, `The bio router in the closet`, `CRISPR i think`, `encodes`, `cells`, `Boston`); the word list passes the retired name guard.

## Learning from the person's edits

When the person corrects a dictated word before sending, the dictionary learns from it (DD-58, PR-22 on the desktop and `serve`, PR-26 in the terminal). Only short correction pairs ever leave the composer; the raw transcript never does, and nothing is stored on the outgoing message.

### Desktop and `serve` ledger

```ts
// ui/desktop/src/utils/composerDrafts.ts
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
  dictionaryRevision: string;  // the listing ETag seen at insert time; opaque, never a store wide counter
  biasScope: 'public' | 'private'; // for the private draft flag (DD-71)
};

export type ComposerDraft = { text: string; images: ...; files: ...; dictation?: DictatedSpan[] }; // at most 16 spans
```

- The ledger lives in renderer memory only, on the existing `ComposerDraft` [C `ui/desktop/src/utils/composerDrafts.ts` `ComposerDraft`], so it survives remounts. `isEmptyComposerDraft` ignores it; it is never persisted, logged or attached to message metadata.
- One `useLayoutEffect` maps every span through prose changes with `locateEdit` (a single region prefix and suffix diff, with the post edit caret as the hint). A programmatic replacement that the diff cannot place marks the span `tainted`; an undo back to the pre insert text marks it `undone`.
- The three exits that hand the composer's text away, `performSubmit`, `handleInterruptionAndQueue` and `handleSteerFromComposer` [C `ui/desktop/src/components/ChatInput.tsx`], are factored into `releaseComposerText(reason)`. `/crew` and `/diverge` drop the ledger. The message editor does the same at `handleSave` in `UserMessage.tsx`.
- Pairs are posted only after the send is accepted, with `userActionHeaders()`. A cancelled send posts nothing. The request names the chat the send went to, including the chat a Home send just created; `null` only when no chat id is known.

### Extraction filters

Two stages. `utils/dictationLedger.ts` holds the span text, so it computes the word level LCS between `inserted` and `current` for each `edited` span and drops a span whose edit is a rewrite (more than half the words changed). `crates/biorouter-dictation/src/recognize/learn.rs` is the authority and checks every pair again on the daemon or in the TUI:

| Filter | Rule |
|---|---|
| Size | each side at most 3 words and 64 characters; no newline; no comma in the source |
| Similar | normalized Levenshtein distance 0.65 or less, or equal step 3 keys, or Double Metaphone codes within one edit |
| Term like target | an inner capital, a letter and digit mix, all capitals of length 2 or more, or absent from the word list |
| Not a language edit | not an inflection, a contraction expansion or a function word change |
| Not an identifier | not a URL, email, path, hash, hex or digits only id ([Validation](#validation)) |
| No secret or PII | `secret_output::redact_text` returns `None` and `guardrails::pii::PiiDetector::scan` finds nothing, for both sides [C `crates/biorouter/src/guardrails/pii.rs:227`]; run in the route and in the CLI, which depend on the core |
| Volume | at most 3 pairs per message |

A shared table of golden pairs, `crates/biorouter-dictation/src/recognize/learn_cases.json`, is read by the Rust tests and by Vitest, so the two stages agree. A `rejected` pair is produced when a change from `changes` was reverted (the text at its position equals `from` again, for example `SPOKE` back to `spoke`).

### The observations route

`POST /dictionary/observations` with `{session_id | null, observations: [{record_id, kind, from, to, entry_id?}]}` ([Routes](#routes)). The handler checks `session_reach` for a named chat before touching the session and does nothing, silently, on a refusal; stamps through `chat_reach` over the chat's stored classification and the live agent's bound provider (Private unless it answers Public), or Private when the request names no chat ([Who stamps the tier](#who-stamps-the-tier)); does nothing, silently, when that stamp is a tier the caller cannot list; drops secret and PII pairs; and is idempotent on `(record_id, from, to)`, so a resend or a give back never counts twice. It always answers 204.

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

ACP, Jupyter AI, **Dictate to clipboard** and Crew's boxes: Biorouter cannot see the person's edits there, or the text is colleagues' (DD-58, DD-83).

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

The person's setting `settings.agent_writes` decides (DD-53, O-3). Only the person raises it ([Authority on each surface](#authority-on-each-surface)).

| Mode | Page label | A term lands | Above the caps |
|---|---|---|---|
| `suggest` (default, O-3) | **Wait for my review** | `suggested` | not recorded beyond 200 pending |
| `on_request` (memory's own protocol) | **Add when I ask, suggest otherwise** | `active`, `reviewed: false`, only when the call sets `requested_by_user` and the term occurs (by fold key, at word boundaries) either in the latest person written user message of the calling chat or in the assistant message that this person message directly answers (the person's agreement to an offer that named it). That person message's provenance must not be `AgentInjection` or `SpawnContext` [C `crates/biorouter/src/conversation/message.rs:705`]. Otherwise `suggested` | `suggested` |
| `add` | **Add without asking** | `active`, `reviewed: false` | `suggested` |

Memory's instructions tell the model to write when the person asks, and otherwise to offer and write after the person agrees [C `crates/biorouter-mcp/src/memory/mod.rs` `base_instructions`]. The offer arm is what makes `on_request` that protocol: in memory's commonest flow the agent asks "Shall I add EPAS1?" and the person answers "yes", a message that does not contain the term.

In every mode, ACP sessions only suggest. Unreviewed active rows bias dictation like any active row, show **Keep** and **Undo** on the tool row, and sit in the **Added by the agent** group of the page. The capability is the memory precedent's machine wide shape without a per call approval card: a card per call would be approval fatigue in every Auto mode turn, ACP answers it without a proven person, and a `serve` daemon would have to withhold the tool.

### What an agent term may carry

`written`, `kind` and `reason` only, with `match = exact`, in every mode. Never `spoken_forms` or `misrecognitions`: an alias is a replacement rule that would rewrite the person's own future words (DD-54).

### Where agent terms may come from

From the conversation the agent sees, not only from text the person typed (O-31, DD-55). The mitigations: exact match only, no agent aliases, the caps per tier, tombstones, the validator and the secret scan, the default `suggest` mode, and instructions that forbid secrets, URLs, identifiers and text from tool results or web pages. If the owner chooses the narrower rule, `add_terms` refuses a term that does not occur in a person written message of the calling chat; that check is the provenance test of `on_request` (person written, not `AgentInjection` or `SpawnContext`) applied to every call, without the offer arm.

### Stamps, dedupe and the result

1. The stamp is the call's `McpMeta.capability` (Public or Private). Each tool takes the dispatch metadata only.
2. A term whose `fold_key` matches a row of the stamp tier bumps that row's `seen` (when the session differs from its `suggested_in`) and creates nothing. A term whose `fold_key` matches a public row seen by a private caller writes nothing, so a private chat never moves a public row.
3. A term tombstoned in the stamp tier writes nothing; it is counted as recorded, so the agent is not told what the person dismissed.
4. The caps are counted in the stamp tier only.

Result texts, constant in shape, with `<n>` counting the valid terms of the call in each outcome:

- MSG-TOOL-SUGGESTED: "Recorded `<n>` suggestions for the person's review in Dictionary. Nothing changes in dictation until they accept."
- MSG-TOOL-ADDED: "Added `<n>` terms to the dictionary. They are marked for the person's review."

A call whose terms all land one way answers that sentence; a mixed call answers MSG-TOOL-ADDED then MSG-TOOL-SUGGESTED. On ACP, Jupyter and the CLI, MSG-TOOL-REVIEW-HINT follows: "Review them with `biorouter dictionary review` on this computer, or in Dictionary in Biorouter." Each entry point declares its surface once at start (`dictation::set_process_surface(Surface::Cli | Surface::Acp)`; the daemon is the default), which also forces suggestions on ACP. The result also carries a user audience content part (`Content::text(…).with_audience(vec![Role::User])`, a pattern `developer__text_editor` already uses beside its assistant audience part [C `crates/biorouter-mcp/src/developer/text_editor.rs`]) holding `{"dictionary_rows": [{"id", "written", "status", "reviewed"}]}` for the tool row's buttons; the model never sees it.

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

1. At the first speech model install, the setup popover and **Settings > Models > Speech models** show **Suggest dictionary words from my chats**, ticked by default and visible beside **Install** (O-33), with MSG-SETUP-SUGGEST below it: "The agent and Biorouter can propose terms they see in your chats. Biorouter also looks for terms in every message you write, on this computer. Nothing is added until you accept it." The middle sentence is left out when `learn_from_chats` starts off (the M-21 gate failed). In `on_request` or `add` mode the last sentence reads "You can change how the agent's terms are added on the Dictionary page." The box starts ticked while `--yes` leaves the capability off, for one reason: the person sees the box beside the button they press, and a script that passes `--yes` never shows the question.
2. `POST /dictation/models/install` carries `{model}` only. After a verified install, the renderer sends the box's value with `POST /dictionary/activation {suggest_from_chats}` and `userActionHeaders()`, the route the Dictionary page's switch also uses. The handler calls `crates/biorouter/src/dictation/activation.rs`, which writes `dictation_dictionary_activation_version: 1` (hidden in `ConfigSettings.tsx`) whatever the value, and enables `dictionary` for new chats only when it is true. Existing chats keep their roster. On a keyed daemon the request needs the proof. On a keyless daemon `true` is refused like raising `agent_writes`, with MSG-DICTIONARY-HOST-SETTINGS, and the box is disabled before the click from `authority.raise_settings`; `false` is admitted and writes the marker. Harvesting turned on by any holder of a `serve` daemon's secret would not be the person's choice (DD-56).
3. At a terminal, `biorouter dictation install` asks "Suggest dictionary words from your chats? [Y/n]" and calls `activation.rs` in process. `--yes` without `--suggest-from-chats` leaves the capability off; `--no-suggest-from-chats` says so explicitly.
4. Once the marker exists, the box is not shown again and nothing turns the capability on by itself; a later "off" sticks. Nothing turns it on at daemon or CLI start. A person who installed a model before PR-23 uses the switch in the Dictionary page's settings area, which sends the same request, or `biorouter dictionary settings --suggest-from-chats on` at a terminal.

The capability's row in **Settings > Chat** toggles it through `POST /config/extensions`, which takes the daemon secret alone at `main` [C `crates/biorouter-server/src/routes/config_management.rs` `add_extension`]. On the desktop that is the person's act; on any daemon it is also open to a holder of the secret. This plan leaves that general route as it is and records it as a residual.

The capability counts toward delegation condition 5 only when enabled, like any other extension.

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
| Machine wide store | every read, write and delete needs the person through `GlobalMemoryInspector` (issue #63) [C] | `suggest` by default: the person's accept is the consent |
| Instructions | act when the person asks; otherwise offer and wait for agreement [C `base_instructions`] | `on_request` mode is that protocol: a term the person asked for, or agreed to when offered, lands at once; instructions forbid secrets, identifiers and tool output |
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
| ACP and Jupyter AI | the tool answer names `biorouter dictionary review` on that computer; Jupyter users run it in a JupyterLab terminal or use `serve`'s sidebar on the same host |

### Files and hand synced lists

Updated in PR-23: `ui/desktop/src/utils/toolGlyph.ts` `BUILT_IN_EXTENSION_KEYS` and its test; `components/settings/capabilities/capabilities.ts` and its test; `components/ToolCallWithResponse.tsx`; `ConfigSettings.tsx`; `crates/biorouter-acp/src/server.rs`; `routes/apps.rs`; new `docs/extensions/built-in/dictionary.md` and a row in [the bundled extensions index](../extensions/built-in/README.md).

**Acceptance.** PR-23: an unknown `tier` field is rejected; the stamp follows `McpMeta.capability`; `list_terms` omits rows outside the caller's tier and reports no count; an 11th term per call is refused, and the 31st per session and the 201st unreviewed land suggested; `suggest` never produces an active row; `on_request` activates a term with `requested_by_user` only when it occurs in the latest person written message or in the assistant message that message answers: an assistant offer naming EPAS1, then the person's "yes", then `add_terms` with `requested_by_user` lands EPAS1 active and unreviewed, with **Keep** and **Undo** on the tool row, while a term named in neither lands suggested; ACP stdio always suggests; `remove_added_terms` cannot touch a person's row; agent terms never carry aliases; `an_agent_cannot_enable_the_dictionary_capability_in_any_chat` finds `workspace_set_tools`, `workspace_open`, `WorkspaceServices::start_session` and `manage_extensions` each refused; the tool is absent in Crew scoped, app, ACP `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; an app manifest naming `dictionary` gets `UnarmedExtension::Platform`; `/agent/call_tool` with `dictionary__add_terms` is refused; an `execute_code` script call of `dictionary__add_terms` or `dictionary__remove_added_terms` is refused, also when the script itself ran through `/agent/call_tool`; instructions contain no term text; the capability is off in a fresh configuration, the first install writes the marker whatever the box says, and only a ticked box (or the settings switch) turns it on; `POST /dictionary/activation` needs the proof on a keyed daemon, and on a keyless one admits `false` and refuses `true` with MSG-DICTIONARY-HOST-SETTINGS; `capabilities.test.ts` and `toolGlyph.test.ts` are green.

## Automatic extraction from chats

`crates/biorouter/src/dictation/extractor.rs` (PR-24) proposes term shaped tokens from the person's own messages, with no provider call (DD-57).

### Hook points

```rust
pub async fn observe_user_text(
    session: &Session,
    classification: SessionClassification, // the cached post ratchet classification
    provider_tier: ProviderTier,           // the bound provider's tier
    text: &str,
);
```

1. In `reply_with_computer_use_scope` [C `crates/biorouter/src/agents/agent.rs:10002`], right after the user message is persisted in the `Ok(None)` arm of the command check. Every consumer awaits `Agent::reply`, so the desktop, `serve`, the CLI front ends and `run` are covered.
2. At `/interrupt` admission in `crates/biorouter-server/src/routes/reply.rs`, for a steer that is not a duplicate (`try_queue_soft_interrupt_keyed` [C]).

### When it runs

All of these hold: the session's roster has `dictionary` enabled (`ExtensionManager::is_extension_enabled`), `settings.learn_from_chats` is true, managed policy allows the dictionary and agent writes, and `Agent.dictionary_learning` is true.

### What it reads and skips

It reads only person written user messages. It skips: provenance `AgentInjection` or `SpawnContext`; every ACP session (stdio and `--ws`), because the Jupyter persona delivers other people's messages as user messages [I]; Crew scoped, app, SubAgent, Scheduled, Hidden and Terminal sessions (a Terminal session's message carries shell history); slash commands; tool output and assistant text, which never reach it. Protected spans of correction step 1 (code, URLs, paths) are skipped.

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

### What it writes

`suggested`, `source: auto`, stamped through `chat_reach` over the cached classification and the bound provider (Private unless it answers Public, [Who stamps the tier](#who-stamps-the-tier)). A new candidate gets `seen: 1` and `suggested_in` = the session. A sighting from a different session than `suggested_in` raises `seen` and moves `suggested_in`. Rows with `seen` below 2 are never listed, so a term appears for review only after the person wrote it in two different chats. The text is never logged.

### Performance budget

Candidate finding is pure and takes microseconds. The write uses `try_write_within(50 ms)`; on contention or timeout the observation is dropped. It is awaited inline, so a short `biorouter run` keeps its observation. Target: p99 under 50 ms with a 5,000 entry store and two writers.

### The M-21 gate

`learn_from_chats` ships on only if an offline run over the person written messages of 200 of the owner's sessions reaches precision 0.8 or more at `seen ≥ 2`. Otherwise PR-24 sets `LEARN_FROM_CHATS_DEFAULT = false`, records the reason, and the page shows **Also find terms in my messages automatically** off. Suggestions bias nothing until accepted, so low precision costs the person review time, never recognition quality.

The run reads the owner's own chats, which may hold patient data, so it needs the owner's consent and someone's labelling time, which is part of owner decision O-23. The recommended default: the owner runs it on their own machine; the owner, or a person they name, labels only the candidate tokens, never whole messages; sessions on private models are excluded, and any other session may be excluded; nothing leaves the machine. If the owner declines, `LEARN_FROM_CHATS_DEFAULT` is false and the reason is recorded.

**Acceptance.** PR-24, driven through `Agent::reply` for the desktop and CLI consumers: `AgentInjection` provenance and SubAgent, Scheduled, Terminal, Crew scoped, app, ACP stdio and `--ws` sessions produce nothing; a private chat produces a private suggestion; a token `PiiDetector` flags is skipped; `seen` counts once per session; p99 under 50 ms with a 5,000 entry store and two writers.

## The left sidebar Dictionary page

PR-20 builds the page on the desktop behind `isBrowserSurface()`; PR-21 removes that gate for `serve` (DD-67, DD-68, DD-94).

### Placement and reveal rules

- `componentItems` in `ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx` [C] gains `{type: 'item', path: '/dictionary', label: 'Dictionary', icon: ENTITY_ICONS.dictionary}` after **Knowledge** and before **Built apps**. The comment "The six destinations behind the `Components` disclosure." becomes seven.
- `components/icons/app-icons.tsx` gains `export const Dictionary = light(_BookA)` (`BookA` exists in the pinned `lucide-react` [C]); `EntityKind` and `ENTITY_ICONS` in `components/icons/entity-icons.ts` gain `dictionary`. The glyph is not reused for memory or knowledge.
- `App.tsx` gains the route before `*`.
- The Components disclosure is collapsed by default [C `readStoredComponentsExpanded`]. After the first verified model install the page writes `biorouter:sidebar-components-expanded` = `true` once and sets `biorouter:dictionary-revealed`; a later collapse sticks.
- While suggestions or unreviewed agent rows wait, the row shows their count and a collapsed disclosure shows a dot. The sidebar reads the count with `GET /dictionary/entries?status=suggested` and `If-None-Match`, on window focus and every 60 s while visible, only when `status.policy.dictionary.allowed` is true.
- The setup popover, Settings and the tool row link to `/dictionary`, which reveals the group by the existing current route rule.
- The row is hidden when `status.policy.dictionary.allowed` is false.

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

### Page layout

The Skills page is the template [C `ui/desktop/src/components/skills/SkillsView.tsx`]: `MainPanelLayout`, `ReadableContent size="chat"`, and a `PageHeader` band with the title **Dictionary**, the count, an InfoTip, `FilterInput` ("Filter terms", ⌘F), a ghost **Export…**, one primary **Add** menu (**Add a term…**, **Paste a list…**, **Import a file…**), and a `⋯` menu with **Delete all terms…**. Rows are `biorouter-list-row` in `br-rows-flat`, with `RowActions` and `RowContextMenu`; skeletons show only for an empty first load. The only accent is the band's **Add** button. Copy lives in `components/dictionary/copy.ts`; load bearing CSS in `components/dictionary/dictionary.css`.

### Groups

| Group | Rows | Shown |
|---|---|---|
| **Suggested** | `status: suggested` (auto rows only at `seen ≥ 2`) | at the top, only when present, with **Review all…** |
| **Added by the agent** | active `agent` rows with `reviewed: false` | only when present |
| **Added by you** | active `user` rows | always |
| **From your chats** | active `agent` rows kept by the person, and active `auto` and `correction` rows | when present |
| **Imported** | active `import` rows | when present |

A query in **Filter terms** collapses the groups into one **Matches** list.

### Rows

Line one is the written form in `text-label`, with the lock badge when private (`withPrivateBadge` [C `components/icons/app-icons.tsx`]; private is a shape, not a tint). Line two is "Said as …" (spoken forms), else the note, else the source label. The meta is "12 uses", shown only when the answer carries usage (private standing).

| Row | Primary `IconAction` | Menu |
|---|---|---|
| Active | **Edit** | **Edit…**, **Copy term**, **Open the chat it came from** (when `suggested_in` is present and reachable through `session_reach`), **Make public…** or **Make private…**, **Delete…** |
| Suggested | **Accept** | **Edit…**, **Copy term**, **Open the chat it came from**, **Dismiss** |
| Added by the agent, unreviewed | **Keep** | **Edit…**, **Copy term**, **Open the chat it came from**, **Remove** |

**Make public…** shows MSG-PUBLICIZE-WARN first: "This term came from a private chat. Making it public lets it appear when you dictate into chats on public models." with **Make public** and **Cancel**.

### Dialogs

| Dialog | Content |
|---|---|
| `TermDialog` (`ModalShell` md) | **Term**; **Spoken as** (one per line, at most 8); **Note**; checkbox **Only in private chats** (ticked by default when the add starts from a private chat, O-6, or from a transcript whose chat cannot be resolved; unticking a private term is a publicize and shows the warning; disabled before the click with `authority.add_private.reason` when the caller has no private standing); a **More options** disclosure with **Match** (**Exact spelling only**, **Also fix near misses**, **Help recognition only**, for `exact`, `fuzzy` and `prompt_only`) and **Always use in dictation** (`bias: always`). Validation texts from [Validation and cap texts](#validation-and-cap-texts); MSG-SAME-LETTERS as a note under **Term** |
| `AddTermsDialog` (`ModalShell` lg) | paste or drop (`.txt`, `.csv`, `.tsv`, `.json`); the daemon's dry run shown as "12 new · 3 already in your dictionary · 1 skipped", skipped lines listed with their reason; **Only in private chats**, disabled before the click with `authority.add_private.reason` when the caller has no private standing; merges only, never deletes |
| `ReviewSuggestionsDialog` | a checklist of suggestions and unreviewed agent rows with **Accept selected** and **Dismiss selected** |
| Export | when private rows exist and the caller has standing, MSG-EXPORT-PRIVATE: "This file leaves Biorouter. It will include your private terms." with **Export with private terms** and **Export public terms only**; the file is `biorouter-dictionary.json` |
| **Delete all terms…** | a typed confirmation (type `delete`); deletes every row this caller can list through `POST /dictionary/entries/delete`, so on `serve` only the rows that browser can see |

### Settings area

At the foot of the page:

| Control | Writes | Notes |
|---|---|---|
| **Agent's terms**: **Wait for my review**, **Add when I ask, suggest otherwise**, **Add without asking** | `agent_writes` | raising is disabled before the click when `authority.raise_settings.allowed` is false, with its reason |
| **Suggest dictionary words from my chats** | `suggest_from_chats`: the `dictionary` capability for new chats, through `POST /dictionary/activation` and `activation.rs` | turning it on is disabled before the click when `authority.raise_settings.allowed` is false |
| **Also find terms in my messages automatically** | `learn_from_chats` | off with the M-21 reason when the gate failed; turning it on is disabled before the click when `authority.raise_settings.allowed` is false |

### On `serve` and remote backends

- `serve` (keyless): listing by standing; add, accept, dismiss, edit, delete and import work on rows the browser can list, stamped with its standing; **Only in private chats** is disabled before the click without private standing (MSG-DICTIONARY-HOST-PRIVATE); **Make public…** and **Make private…** are disabled before the click with MSG-DICTIONARY-HOST-TIER, with the term filled in (`publicize` or `privatize`); raising **Agent's terms**, and turning **Suggest dictionary words from my chats** or **Also find terms in my messages automatically** on, are disabled before the click with MSG-DICTIONARY-HOST-SETTINGS, which names `biorouter dictionary settings` with the option filled in; "12 uses" appears only with standing.
- A desktop app on a keyed external backend without that server's key: public rows only; every write control is disabled before the click with MSG-REMOTE-NO-KEY (in the variant O-17 chose); with the key, the page behaves as on the desktop.
- Managed `dictionary: false`: the sidebar row is hidden; a visit to `/dictionary` shows MSG-POLICY and the list read only, with **Export…** and delete still allowed.

### Context menu door

In `ui/desktop/src/main.ts`'s `context-menu` handler, Electron's spelling item "Add to dictionary" [C `main.ts:2014`] becomes **Add to spelling dictionary**. A new item **Add to Biorouter dictionary…**, shown for a misspelled word or a selection of at most 64 characters, sends `dictionary:add-term` to that window with `{text, x, y}` (the menu's `params.x` and `params.y`), which opens `TermDialog` prefilled (DD-68). Main knows the window, not the pane, so the renderer resolves the origin: it walks up from the element at that point to the nearest container that names a local chat (a split pane, History, the artifact panel of a pane). **Only in private chats** then starts ticked when that chat is private, or when the point sits in a transcript whose chat cannot be resolved, such as a shared transcript (fail toward private); outside any transcript it starts unticked (O-6). Without the origin, a word selected in a private chat's pane while a public chat was current would be filed public with no warning, because nothing was unticked. Terms are never copied into the OS spellchecker.

### Tool row controls

In `components/ToolCallWithResponse.tsx`, for a `dictionary__add_terms` row, from the result's user audience part: **Add to dictionary** on suggested rows (accept); **Keep** and **Undo** on unreviewed active rows (`POST /dictionary/entries/keep`; `DELETE /dictionary/entries/{id}`, which writes the person's tombstone); a **Review in Dictionary** link.

### Empty state

MSG-EMPTY: "No terms yet. Add the words dictation should spell your way, such as UCSF or EPAS1." with the **Add** menu.

### Files and guard updates

New `components/dictionary/{DictionaryView,DictionaryRow,TermDialog,AddTermsDialog,ReviewSuggestionsDialog}.tsx`, `useDictionary.ts`, `parseTermList.ts`, `copy.ts`, `dictionary.css`. Guard files changed in PR-20: `landing/app-mockups.js` (so `node landing/scripts/check-consistency.mjs` passes), `tests/e2e/helpers/sidebar.ts` `COMPONENT_LABELS`, `styles/measures.test.ts`, `settings/settingsVocabulary.test.ts`, `App.routing.test.tsx`, `Layout/AppLayout.test.tsx`, `BioRouterSidebar/AppSidebar.test.tsx`; `main.ts` and `preload.ts` for the context menu. The sidebar geometry rules of [the sidebar chat list](../desktop-ui/sidebar-chat-list.md) and the fill rules of [shared controls](../desktop-ui/shared-controls.md) apply unchanged.

**Acceptance.** PR-20: `parseTermList.test.ts`; `DictionaryView.test.tsx` (groups, review rows); `useDictionary.test.ts` (`userActionHeaders()` on every call, stale responses dropped); `dictionaryRows.browser.test.tsx` (no row moving under a hovering pointer); the disclosure opens once after the first install and a later collapse sticks; a main process test of the menu labels and of the `{text, x, y}` payload; a renderer test that a selection in a private chat's pane, while a public chat is current, opens `TermDialog` with **Only in private chats** ticked, and that one in an unresolvable transcript does too. On a fresh profile, installing a model reveals **Dictionary**, and a term added there biases the next dictation into a matching chat (a recorded manual row).

## Dictionary commands in the CLI

### Commands

`crates/biorouter-cli/src/commands/dictionary.rs` and `Command::Dictionary` in `cli.rs` (PR-13). They work offline, directly on the store under its lock; no daemon is needed (DD-75).

```text
biorouter dictionary list      [--format text|json|csv] [--status active|suggested|all]
biorouter dictionary add       TERM… [--spoken "…"] [--private]
biorouter dictionary edit      TERM [--term NEW] [--spoken …]
biorouter dictionary remove    TERM… | --id ID
biorouter dictionary review
biorouter dictionary accept    ID…
biorouter dictionary dismiss   ID…
biorouter dictionary import    FILE|- [--format txt|csv|tsv|json] [--dry-run]
biorouter dictionary export    [FILE|-] [--include-private]
biorouter dictionary publicize TERM
biorouter dictionary privatize TERM
biorouter dictionary settings  [--agent-writes suggest|on_request|add] [--learn-from-chats on|off]
                               [--suggest-from-chats on|off]
biorouter dictionary clear     [--suggested-only]
```

- Every command prints the account and the store folder it acts on to stderr, so stdout stays clean for `list` and `export`.
- `--spoken` takes a comma separated list and may repeat. `add` stamps public unless `--private` (O-6: a terminal add starts from no chat).
- A `TERM` names every row with that exact written form that the caller can list. When it names more than one (twins), the command lists them with ids and asks which, or takes `--id`.
- `review` shows suggestions and unreviewed agent rows in a multiselect (written form, source, lock for private, reason); the selected rows are accepted or kept, then it asks "Dismiss the rest? [y/N]".
- `publicize` and `privatize` show MSG-PUBLICIZE-WARN (publicize) and then ask the person to type the term exactly (O-5). Only then is `UserDictionaryTierChange` constructed.
- `clear` prints "This deletes `<n>` terms from `<folder>`." and asks the person to type `delete <n>`. `--suggested-only` deletes suggestions only.
- `import` takes the format from the file's extension (`.txt`, `.csv`, `.tsv`, `.json`); `--format` overrides it, and standard input (`-`) needs it unless the content is the export JSON.
- `settings` with no flag prints the three settings. A flag that raises `agent_writes`, or turns `learn_from_chats` or `suggest_from_chats` on, asks "Change this setting? [y/N]" first; lowering or turning off asks nothing. `--suggest-from-chats` goes through `activation.rs`, as the install box does. This is the host step that MSG-DICTIONARY-HOST-SETTINGS names on `serve`.
- `list --format text` columns: written, status, tier, source, uses, id; `csv` the same; `json` an array of `EntryView`.

### Terminal only rules

| Situation | Reads | Writes | Tier change | Private rows |
|---|---|---|---|---|
| Interactive terminal (`prompt_can_run()` [C `crates/biorouter-cli/src/commands/needs_terminal.rs:49`]), `BIOROUTER_TERMINAL` unset | yes | yes | after the typed confirmation | listed and reviewed; exported only with `--include-private`, `caller_capability()` Private [C `crates/biorouter-cli/src/session/privacy.rs:65`] and MSG-EXPORT-PRIVATE |
| No terminal (pipe, cron, `ssh host cmd`), `BIOROUTER_TERMINAL` unset | public rows | refused (exit 2), except `import` | refused | withheld, with no count |
| An agent's shell (`BIOROUTER_TERMINAL=1`) | public rows | refused with MSG-DICTIONARY-AGENT (exit 2) | refused | withheld, with no count |
| Managed `dictionary: false` | yes | refused with MSG-POLICY (exit 2); `remove` and `export` allowed | refused | as above |

`import` is admitted without a terminal (still refused under `BIOROUTER_TERMINAL=1` and by policy), because the documented sync pipes into it over SSH; it only merges and never deletes. Rows marked `tier: "private"` import private, and the rest import public. Every other write, `settings` included, needs a terminal, the rule DD-76 applies to live capture, as accident prevention and not a boundary. Private rows follow DD-37 at an interactive terminal with `BIOROUTER_TERMINAL` unset: `list` and `review` show every row. Only `export --include-private` also needs `caller_capability()` Private, the part of DD-77 that applies here.

### In a session

In the TUI and the classic REPL, `/dictionary add <term>` and `/dictionary review` are intercepted in `session/input.rs` beside `/folder-access` [C `crates/biorouter-cli/src/session/input.rs`] and handled in `session/mod.rs` and `session/tui/mod.rs` `handle_slash`; they never reach the model (PR-26). `/dictionary` alone prints the two forms and the counts. `add` defaults to the session's tier. `review` is the TUI modal or a `cliclack` multiselect, with local human authority (`DecisionAuthority::from_local_human_surface()` [C]). When suggestions wait, the session starts with one quiet line: "3 dictionary suggestions; /dictionary review".

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
      "match": "exact", "note": null, "tier": "private" }
  ]
}
```

No ids, session ids or usage. Private rows only after MSG-EXPORT-PRIVATE, each carrying `"tier": "private"` so an import elsewhere restamps it private.

Import accepts:

| Format | Shape |
|---|---|
| `lines` (`--format txt`) | one term per line; `EPAS1 = epas one, e pass one` adds spoken forms; `#` starts a comment |
| `csv`, `tsv` | header `written,spoken_forms,note`; spoken forms separated by `;` inside the field |
| `json` | the export shape above |

The plan for each row is `new`, `merged` (union of spoken forms and misrecognitions into the existing row), `duplicate` or `invalid` with its code. Import never deletes. Imported misrecognitions start at `count` 2, since the person supplied them. `--dry-run` prints the plan.

### Syncing two hosts

Two hosts have two dictionaries (a laptop and an SSH server, or a Windows desktop and its WSL account). The only sync is a pipe the person runs, the one MSG-SYNC-HINT shows ("To use your laptop's dictionary terms here, run on your laptop:" followed by the command):

```bash
biorouter dictionary export - | ssh <host> biorouter dictionary import -
```

On a systemd host, the receiving side runs as the service account (`sudo -u biorouter -H biorouter dictionary import -`). Adding `--include-private` on the exporting side carries private terms after the confirmation; they import private.

**Acceptance.** PR-13: every write except `import` is refused without a terminal (exit 2); every write, `import` included, is refused under `BIOROUTER_TERMINAL=1`; a piped `import -` merges; `settings` asks before raising and not before lowering; a piped `list` withholds private rows with no count, and `list` at an interactive terminal shows them on a machine configured for a public provider; `publicize` needs the typed phrase; `UserDictionaryTierChange` built in exactly two files; a term added in one terminal is listed by a second process at once. PR-26: `/dictionary add` and `/dictionary review` never reach the model.

## Store protection and boundaries

### The path inspector

New `crates/biorouter/src/security/dictionary_store.rs` (PR-13), modelled on `security/session_store.rs` [C]: `is_in_dictionary_store(path)`, a `DictionaryStoreInspector` registered beside `SessionStoreInspector` in `Agent::create_tool_inspection_manager` [C `agent.rs:6264`], and an `uninspected_boundary_refusal(tool_name, arguments, boundary)`. It refuses tool reads and writes of `<config>/dictionary/` for callers of both tiers, using the same path resolution as the session store inspector (DD-42).

### Boundary refusals

`POST /agent/call_tool` dispatches with `CallCapability::public_enforced()` on the secret alone [C `crates/biorouter-server/src/routes/agent.rs:3398`], so a private chat's model holding the secret could otherwise land public stamped rows. `dictionary_store::uninspected_boundary_refusal` refuses `dictionary__add_terms` and `dictionary__remove_added_terms` at both uninspected doors:

- `call_tool_boundary_refusal` [C `routes/agent.rs:3320`], chained after the global memory, session store and knowledge delete refusals;
- the `execute_code` chain's `uninspected_boundary_refusal` [C `crates/biorouter/src/agents/code_execution_extension.rs:3569`].

The script door refuses both tools even from a private chat, as DD-42 says. A script's inner calls inherit the capability its `execute_code` call was admitted with [C `code_execution_extension.rs` `get_tool_infos`], and the inspector chain does not reach a script that a person ran through `POST /agent/call_tool` [C `uninspected_boundary_refusal` doc comment]. Such a script runs as `public_enforced()`, so a private chat's term added from it would land public. The agent calls `dictionary__add_terms` directly instead; `dictionary__list_terms` stays callable from scripts.

### Managed policy keys

The managed `dictation:` block ([Engine and models](engine-and-models.md) and [managed policy](../security/managed-policy.md)) has two dictionary keys (DD-90). Each only narrows; an unknown key, a wrong type, an untrusted file or a failed load turns dictation off.

| Key | Effect | Choke points |
|---|---|---|
| `dictionary: false` | no dictionary edits, sidebar row, capability, extractor or bias | every `/dictionary/*` write and observation (MSG-POLICY); `add_extension_with_origin` (a resumed roster cannot load the capability); `permission_for` denies `dictionary__*` [C `crates/biorouter/src/managed/mod.rs`]; the managed deny arm in `call_tool_boundary_refusal`; the extractor; every CLI write (exit 2); `bias_set` returns `EligibleEntries::empty()`. Export and delete stay allowed |
| `dictionary_agent_writes: false` | the agent's tools are not offered; the extractor is off; `agent_writes` pinned to `suggest` | `permission_for`; the roster; `POST /dictionary/settings` refuses raising; the person's manual edits and the correction learner still work |

### What a shell can still do

A shell can read `dictionary.json` directly, because DR-17 left the filesystem open. The inspector stops a cooperative agent's file tools and named paths, not a determined actor. On a keyless daemon a shell that recovers the daemon secret (`ps eww`) can also make every dictionary write a `serve` browser may make and install a pinned model ([What is and is not claimed](#what-is-and-is-not-claimed)). These are recorded as residuals, as accident prevention.

**Acceptance.** PR-13: `cat` and `text_editor view` of the store refused for both tiers. PR-23: `/agent/call_tool` with `dictionary__add_terms` refused. PR-23, with the policy tests of PR-05: a resumed roster does not load `dictionary` under policy.

## Privacy invariants

| Id | Invariant | Named tests |
|---|---|---|
| P-1 | Audio stays in memory in Biorouter's processes on the transcribing host: no temp file (the Jupyter handler pipes stdin), no log line, no diagnostics entry, no session row; the stream buffer is zeroed at finish, cancel or the 60 s expiry; no off machine engine in v1. A reverse proxy in front of `serve` is the operator's: `browser-access.md` names the setting that keeps request bodies in memory (`client_body_buffer_size 512k` or `proxy_request_buffering off` on nginx, whose stock configuration wrote every chunk above 16 KiB to disk [M]) | `dictation_routes.rs` (no new file in the sandbox root; tracing capture has no audio derived content); buffer zeroed on drop (unit); Jupyter handler test (no file under `TMPDIR`); `no_cloud_audio_routes.rs`; the `serve` job's nginx log has no "buffered to a temporary file" line |
| P-2 | Transcripts live only in the composer: never in notifications (counts only), logs, `messages.metadata_json`, telemetry or the worker's stderr | tracing capture in `dictation_routes.rs`; the `UserMessage.tsx` grep test; notification tests in `dictationAnywhere.test.ts`; the OpenAPI diff shows no dictation field on `Message`; the log callback test |
| P-3 | One bias set feeds every channel | the `bias.rs` table test; `eligible_entries_are_built_only_by_bias_set` |
| P-4 | The daemon decides the target | `dictation_routes.rs`: a refused `session_id` gives public scope and an identical response; an unproven caller's `new_chat_provider` is ignored; a `new_chat` stream takes the tier the bind would produce, so an `ollama` cloud tag, a remote `OLLAMA_HOST` and a private worker under a public lead give public scope |
| P-5 | Public by default: Crew, clipboard, app preview and `dictate` without an admitted `--session` take private terms only for one recording a person confirms after MSG-PRIVATE-TERMS, never for an agent. Public only: Jupyter, ACP and `transcribe` without an admitted `--session` | `bias.rs` rows for `crew` and `external` with and without `include_private_terms`; `include_private_terms` ignored without the proof; CLI tests (`--private-terms` refused under `BIOROUTER_TERMINAL=1` and without a terminal); the Jupyter handler passes no session |
| P-6 | No existence oracle | `dictionary_oracles.rs` (including the keyless `private: true` add from a caller without private standing, byte identical for an existing and an absent private term); the echo oracle test in `dictation_routes.rs`; caps per tier (unit); `transcribe --format json` has `bias_scope` and no count |
| P-7 | A cooperative agent, and any caller on a keyed daemon without the proof, never opens a microphone, transcribes for itself through a capture path, installs, removes or selects a model, accepts or dismisses suggestions, edits or deletes a person's row, changes a tier, raises `agent_writes` or turns learning on. On a keyless daemon, any holder of the daemon secret, including an agent shell that recovers it, can install a pinned model and make the dictionary writes a `serve` browser may make on rows it can list; and an agent holding a Biorouter Copilot grant on the person's web browser can press the microphone in a `serve` tab or the Jupyter AI chat (DD-88). Both residuals are recorded in SD-16 and [What is and is not claimed](#what-is-and-is-not-claimed) | `dictation_keyed.rs`; `capture_consent_is_minted_in_one_file`; CLI refusals when `BIOROUTER_TERMINAL=1`; the capability's tool list is exactly the three tools; boundary refusals at `call_tool` and in scripts; the Copilot refusal test; the keyless refusals in `dictionary_no_user_key.rs` |
| P-8 | The person is warned, then free: publicize, export private terms, include private terms in one clipboard, stdout, Crew or app preview recording, send a privately biased draft to a public model or switch to one, dictate to a plain `http:` remote, install turbo on a CPU host | renderer tests for MSG-PUBLICIZE-WARN, MSG-EXPORT-PRIVATE, MSG-PRIVATE-TERMS, MSG-PRIVATE-DRAFT, MSG-PRIVATE-DRAFT-SWITCH, MSG-REMOTE-HTTP, MSG-TURBO-CPU; the `--private-terms` CLI test |
| P-9 | Agent terms carry no aliases and are suggestions by default; the capability is absent from Crew scoped, app, `--ws`, SubAgent, Scheduled, Hidden and Terminal sessions; ACP only suggests; the extractor never runs in ACP | the capability's tests in PR-23; the extractor tests in PR-24 |
| P-10 | The network facing daemon cannot record: `biorouterd` links no capture code; capture starts only behind `CaptureConsent` or a person's gesture in a page | the daemon binary contract in `dictation_routes_auth.rs` (`biorouterd __br-dictation-worker capture` exits 2; the bytes of the unstripped `biorouterd` contain neither `ma_device_init` nor `libpulse.so.0`); `cargo tree -p biorouter-server -e normal,build` holds no `biorouter-dictation-capture`; on the binaries `cross-build-nightly` builds with the release command itself (`cargo build --release --bin biorouterd --bin biorouter`), the exit check and `strings biorouterd \| grep -c libpulse` is 0; the same `strings` check in every package smoke. The release build is checked as well because one Cargo invocation unifies the features of a crate both binaries use, so a test that builds only `biorouter-server` cannot see what the release build links in [I] |
| P-11 | Only pinned bytes are parsed | installer tests (mismatch deletes, `.partial` never loaded, ids only); the self test refuses a non pinned model; a fresh hash after an mtime change |
| P-12 | The worker is a closed box: frames on stdin and stdout only, no socket, daemon secrets stripped, no configuration or log files, exits on end of file, library logs filtered | `dictation_worker.rs` environment scrub; the 300 byte log line test |
| P-13 | Tier changes are the person's | `the_tier_proof_is_constructed_in_exactly_two_files`; the keyless tier refusal |
| P-14 | No private entry text leaves the store except to a caller allowed to list it | instructions contain no term text; a piped `list` withholds private rows; refusal snapshots name no term; a keyless add from a caller without private standing never answers with a private row |
| P-15 | Policy only narrows | the parse isolation test; `dictation_policy_routes.rs`; a resumed roster does not load `dictionary` under policy |

## Related documentation

- [Implementation plan](implementation-plan.md): the requirements, the decision register this document elaborates, and the order of the pull requests.
- [Voice dictation](README.md): the folder index and what each dictation document covers.
- [Engine and models](engine-and-models.md): the worker protocol that carries the bias set, the decoding parameters, the `/dictation/*` routes and the managed `dictation:` block.
- [Surfaces](surfaces.md): the composer, the private draft flag, the setup popover and the full message catalog quoted here by id.
- [Testing and release](testing-and-release.md): where the dictionary's tests run, the accuracy gates and the measurements M-9, M-19 and M-21.
- [Speech model benchmarks](../research/voice-dictation/speech-model-benchmarks.md): the measurements behind the token prompt, the decoder bonus and the correction guards.
- [Dictation tools survey](../research/voice-dictation/dictation-tools-survey.md): how other dictation tools handle vocabularies, aliases and learning.
- [Privacy tiers](../security/privacy-tiers.md): the tier rules, DR-15 and DR-17, that the bias set and the stamps follow.
- [Institutional affiliation](../security/institutional-affiliation.md): the affiliation axis (DR-26) that dictionary rows do not carry, the subject of O-32.
- [Managed policy](../security/managed-policy.md): the policy file that the `dictionary` and `dictionary_agent_writes` keys extend.
- [Memory extension](../extensions/built-in/memory.md): the memory behaviour that R5 compares the `dictionary` capability with.
- [Serve decisions](../deployment/serve-decisions.md): SD-8 and SD-11, the keyless shapes the `serve` write rules follow.
- [Sidebar chat list](../desktop-ui/sidebar-chat-list.md): the sidebar the **Dictionary** row joins.
- [Shared controls](../desktop-ui/shared-controls.md): the fill and control rules the Dictionary page follows.
- [Command reference](../cli/command-reference.md): where the `biorouter dictionary` commands join the CLI reference when they ship.
