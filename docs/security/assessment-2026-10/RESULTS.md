# Defensive security audit results


## Scope and completion

The scoped defensive audit and its specified regression checks are complete. This means that the selected boundaries were reviewed, the recorded findings received source changes, and the defined checks passed. It is not a statement that BioRouter has no remaining security defects. The remediation branch is unmerged and unreleased; the official v1.93.4 distribution has not been replaced by these changes.

The audit used synthetic data and temporary fixtures, assumed that legitimate researchers follow their institutional data-handling instructions, and considered outside inputs and ordinary separate non-root accounts. It used no PHI or real credentials. The documented intent is defensive review and remediation.

## Target and provenance

The runtime baseline was the official BioRouter v1.93.4 Linux/amd64 package, SHA-256 `8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65`, run under Docker Desktop x86 emulation on Apple Silicon. Its release commit is `b824e2c3e802e243804a150aa7c349fa6ce6a8b6`. Source review and component checks used assessment base `f73ebf74c578717a7af64b221e57358a63dda8e3` and the recorded remediation patch. The release-to-assessment-base comparison contained no runtime Rust or Electron changes.

Local Rust regression tests ran on Darwin/arm64. The separate-account post-change check compiled the Unix session-directory helper verbatim and ran it on Linux/arm64 in a restricted container. That helper check used a synthetic file; it did not rebuild the full Linux application or repeat the released application's SQLite scenario. These evidence streams are reported separately.

## Recorded observations

| Evidence stream | Design | Result |
|---|---|---|
| Baseline API and browser-session protocol | 55 prespecified cases × 3; 165 records, 162 exercised | 120 expected denials; 42 valid controls allowed; one control unexercised in each repetition |
| Baseline local accounts and data-store access | 8 cases × 3; 24 observations | 3 synthetic SQLite session-title disclosures; 12 other denials; 9 valid controls allowed |
| Baseline sandbox file APIs | 8 cases × 3; 24 observations | 6 dangling-link writes outside the workspace; 15 other negative cases denied; 3 valid controls allowed |
| Post-change sandbox file APIs | Same 8 cases × 3; 24 observations | All 21 negative cases denied; all 3 valid controls allowed; no outside writes |
| Baseline scripted components | 14 observations; one run | 2 optional-judge approval-boundary observations; 4 blocks; 3 controls; 5 diagnostic/inconclusive records; no tool dispatched |
| Post-change public editor | 11 checks × 3; 33 observations | All 21 unsafe-path cases denied; all 12 valid-use controls allowed; no outside files or directories created |
| Post-change session-directory guard on Linux | 3 two-UID synthetic fixtures and 2 refusal checks | Observer denied and owner access retained in all 3 fixtures; foreign-owned and final-symlink directories refused |
| Post-change Rust regressions | 19 tests in 3 approval/session suites; 2 scope/configuration tests | All passed; includes owner SQLite creation/read/reopen, invocation scope, malformed/provider-failure handling, legacy permissions, and unresolved scoped paths |

The counts are descriptive. The case sets were purposively selected; repetitions reused fixed cases and are not independent trials. Baseline and post-change denominators must not be combined into a general pass rate. API observations were handcrafted protocol requests, not browser-engine trials. Scripted providers did not measure live-model behavior. Historical file-API records embed repetition `1`; the run manifest associates their distinct source logs with repetitions 1–3 without changing raw records.

## Findings, source changes, and verification

### 1. Session-history permissions — issue #445

In the official baseline release, a separate unprivileged UID recovered a synthetic title from the SQLite session store in all three fixtures with a traversable home directory and app-created mode-0755 session storage. The outsider used no daemon credential and did not act as the owner or root. Other tested secret/configuration boundaries held.

The source change opens the real current-user-owned session directory without following its final symlink and applies Unix mode `0700` before opening SQLite. Local application regressions confirmed that new and existing directories become private while the owner can create, read, close, and reopen history. A separate Linux/arm64 helper replay confirmed observer denial and owner access in three two-UID fixtures, plus refusal of foreign-owned and symlinked directories. Full rebuilt-Linux-application separate-account verification remains outside the completed scope. See [issue #445](https://github.com/BaranziniLab/biorouter/issues/445).

### 2. Workspace path handling — issue #446

The baseline sandbox resolver treated dangling links as missing path components. Subsequent writes followed those links through both `LocalProcessSandbox` and the host-side `DockerSandbox` file API, producing six outside writes across the repeated component checks. This was not a container escape.

The source changes reject unresolved symlinks, validate the deepest existing ancestor before patch-related directory creation, account for diff-selected paths and overlap prefixes, and prevent ambiguous paths from inheriting explicitly user-created directory grants. The repeated sandbox replays denied all 21 negative observations while retaining all 3 controls. Public-editor replays denied all 21 unsafe-path observations while allowing all 12 controls. Controls included ordinary writes, nested file creation, a supported diff overlap prefix, and an existing internal symlink used through direct editor write. The additional editor routes were identified through source review and checked after remediation; they were not all separately reproduced against the original release. See [issue #446](https://github.com/BaranziniLab/biorouter/issues/446).

### 3. Optional Smart Approve judge scope — issue #447

The optional `SMART_APPROVE_JUDGE` setting is disabled by default. Baseline scripted diagnostics showed that the judge omitted arguments and could store name-only output as tool-wide automatic approval, including an unsolicited tool name. No tool was dispatched, and no live-model failure was demonstrated.

The source change supplies actual request IDs, names, and arguments; validates returned ID/name pairs against the current candidate set; rejects malformed, unsolicited, or ambiguous classifications; and applies any approval only to the inspected invocation. It creates no persistent tool-wide grant. Legacy automatic allowances are ignored centrally, while explicit user grants and restrictive preferences remain effective. Deterministic regression tests exercised these boundaries, including different operations using the same tool name and a later invocation. The optional classifier still depends on model judgment. See [issue #447](https://github.com/BaranziniLab/biorouter/issues/447).

## Recommendations and remaining limits

Each issue includes recommendations that preserve routine use: protect app-owned history automatically, retain supported file workflows and clear errors, and keep permission decisions scoped to the operation while preserving explicit human grants. All three issue records remain open for maintainer review and release integration.

A race-resistant workspace design using directory-handle-relative operations remains a recommended follow-up. Concurrent path replacement, Windows ACL parity, full Docker command containment, browser-engine behavior, real-model response to deceptive content, all extensions, and all deployment configurations were not assessed. LocalProcessSandbox command execution uses the host account; its file-API results do not demonstrate OS process isolation. Changing directory permissions does not revoke an outsider's already-open descriptors, and same-account processes are outside the demonstrated separate-account boundary.

Tool-output marker detection and untrusted framing were implementation diagnostics. Three of four synthetic inputs matched markers and all four received framing; that does not establish model resistance to embedded instructions. Two synthetic credential forms were redacted while a benign control remained unchanged.

The audit documents bounded evidence and source-level remediation. It does not establish comprehensive security, independent certification, or HIPAA compliance. All fixes remain unmerged, unreleased, and unverified in a newly distributed application build.

Compatibility limit: final-symlink session directories are refused, including manually symlink-relocated stores. The existing constructor uses a panic-based error path for this refusal. A recoverable, actionable startup message and safe relocation support remain recommendations before broad deployment.
