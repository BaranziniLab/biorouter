# Crew evidence records

Curated, text-only records of bounded Crew runs: each names the source revision and artifacts it
ran on and says what it establishes and what it does not. All data in them is synthetic. A record
proves only its own scope on its own build; later commits need their own runs, and none of these
establishes institutional deployment or HIPAA compliance. Raw logs, transcripts and screenshots stay
local and are not published.

| Record | What it holds |
|---|---|
| [UI redesign acceptance, 2026-09](ui-redesign-acceptance-2026-09.md) | The redesign and naming campaign: four live QA rounds with novice critics, per-round security results, the seven final acceptance lanes on `461f7899`, fixture provenance and what remains unverified |
| [Shared-daemon acceptance on `532c3b7d`](shared-daemon-532c3b7d-20260923.md) | Observer fairness and released slots, PTY continuation, membership and derived-source refusals, and the three-user file and `qwen3:8b` replay on the `532c3b7d` native pair |
| [Fresh 5455 shared-daemon acceptance](shared-daemon-acceptance-20260922.md) | The `qwen3:8b` shared-daemon tool workflow, the bounded observer lane and the large observer backlog stress on `5455ebf9` |
| [Queued source ACL observation](queued-source-acl-20260923.md) | One live run of queued-frame source-ACL revocation against the disposable AWS broker |
| [Post-fix NDJSON compression](ndjson-compression-20260923.md) | The daemon's observation stream compression after its fix, on a restarted disposable daemon |
| [Crew CLI observer and context](crew-cli-observer-context-20260922.md) | A fresh three-user CLI fixture: observer and context results, personal MCP post, read and revocation, authority checks and the pinned transfer checks |
| [Synthetic 50-UID broker soak](crew-50-uid-soak-20260922.md) | A standalone 30-minute, 50-account broker workload in a disposable container |
| [Native CLI verification on `7ab40c81`](cli-7ab40c81-20260923.md) | The ordinary debug build and the supported SSH and Unicode-history smoke |
| [Prepared development apps](prepared-gui-7ab40c81-20260923.md) | The three isolated QA apps refreshed to embed the `7ab40c81` pair; not a runtime pass |
| [Hosted CI extraction](ci-dd70051e-20260923.md) | A read-only extraction of the hosted Rust and frontend runs and the PR #366 required-check rollup |
| [Linux ARM64 on `532c3b7d`](linux-532c3b7d-20260923.md) | Linux build, focused suites and ordinary-UID lifecycle for `532c3b7d` |
| [Linux ARM64 on `5455ebf9`](linux-5455ebf9-20260922.md) | Linux build and lifecycle for the published `5455ebf9` |
| [Linux ARM64 refresh on `dd70051e`](linux-dd70051e-20260923.md) | Non-test Linux artifacts rebuilt from a fresh archive of `dd70051e` |
| [Source-only history provenance](source-only-history-20260922.md) | How the published history dropped the screenshot commit without reassigning binary evidence |

## Screenshots kept local

Earlier reports cite synthetic-data captures from isolated Electron development profiles. They
were reviewed for readable controls and visible credentials, but the repository ignores `*.png`
and none of them is published, so they are named here rather than linked. A screenshot proves the
visible state at capture time; it does not establish a model tool result, a completed native Save,
or acceptance on a later binary. The [app evidence](../crew-ui-acceptance-report.md) and the
[matched backend traces](../local-provider-boundary-report.md) draw those distinctions.

| Capture | What it shows |
|---|---|
| `fresh-control-members.png` | Three enrolled participants in the clean restricted channel |
| `fresh-control-result.png` | A rendered read result, separately matched to a typed backend response |
| `bob-personal-crew-grant.png` | Explicit destination and posting consent; the later model request failed |
| `bob-personal-greeting.png` | An existing personal conversation before the Crew grant |
| `bob-slash-existing2.png` | An existing session carried into Crew |
| `carol-blocks-preview.png` | A synthetic image preview; a completed named Save was not established |
| `carol-opaque-uploaded.png` | A downloadable binary file card without an image preview |
| `carol-fresh-pam-after-password.png` | Authentication completion, separate from full broker connectivity |
| `carol-pam-cancelled.png` | Return to the disconnected authentication state |
| `carol-processing-correction.png` | A historical failed agent attempt, retained as a failure |
| `alice-cross-channel.png` | A historical model result rejected for lack of matching retrieval |

Two byte-identical historical copies, `carol-personal-crew-after.png` and
`final-9413-reconnected.png`, also stay local; their canonical image is `carol-agent-final.png`.
The UI redesign campaign's screenshots are under `/private/tmp/crew-ui-redesign/live/shots/` on
the machine that ran it.

## Related documentation

- [Implementation status](../implementation-status.md) — the ledger whose rows these records support
- [Validation report](../validation-report.md) — the chronological command and artifact record
- [Crew research folder](../README.md) — the index of every Crew design, plan and report
