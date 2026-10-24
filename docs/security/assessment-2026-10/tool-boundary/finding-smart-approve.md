# Optional Smart Approve judge decision scope


**Status:** a configuration-specific design weakness demonstrated with a scripted provider; no tool was dispatched and no live-model failure was measured. The unmerged, unreleased assessment branch contains an invocation-scoped correction. Tracked by [issue #447](https://github.com/BaranziniLab/biorouter/issues/447).

## Baseline behavior and configuration

`SMART_APPROVE_JUDGE` is disabled by default. When enabled for tools whose annotations could not establish risk, its baseline input contained tool names without operation arguments. A positive response could be persisted as `smart_approve` tool-wide `AlwaysAllow`. A scripted provider showed that the storage path also accepted an unsolicited tool name. A later operation with different arguments could therefore inherit an automatic decision made for the same name.

The component observations concern representation, permission decisions, and persistence only. They did not execute the tool or establish a live model's response to deceptive content. The default unknown-tool policy requires confirmation; ordinary annotation-based Smart Approve and explicit user grants are separate mechanisms.

## Source correction and deterministic checks

The judge now receives each candidate's request ID, tool name, and actual arguments. Its structured response must identify a supplied request-ID/tool-name pair. Malformed, unsolicited, or ambiguous results grant no permission, and duplicate input IDs are not sent for ambiguous classification. An accepted result applies only to the inspected invocation.

The inspector no longer persists positive judge decisions as tool-wide grants. The central permission getter ignores legacy automatic `AlwaysAllow` entries, so permission settings cannot revive or display them as effective grants. Explicit user `AlwaysAllow`, `AskBefore`, and `NeverAllow` choices remain in their separate tier. Restrictive legacy smart preferences remain effective.

Focused tests verified argument inclusion, a mixed read/write batch using the same tool name, unknown IDs and names, malformed responses, provider failure, duplicate candidate IDs, absence of a new permission-file write, and a fresh decision for a later invocation. Permission-file reload and policy tests verified that legacy automatic grants are ignored while explicit human grants and restrictive preferences are retained.

## Recommendations preserving normal use

Retain automatic handling for known tools according to configured risk annotations and preserve explicit user grants and scoped approvals. For the optional judge, approve only the reviewed invocation and ask the user when classification fails or is ambiguous. This preserves convenience for accepted operations without converting uncertain model output into persistent broad permission.

Keep the judge disabled by default until its deployment policy and live-model behavior are evaluated. The scope correction prevents permission expansion; it does not make the classifier's read-only judgment deterministic or establish resistance to prompt injection. Registered annotations are also inputs to configured policy, rather than proof of arbitrary tool behavior. No general security-success rate can be inferred from scripted verdicts.

## Evidence locations

- `tool-boundary/raw/core-and-regressions.log`: baseline scripted component observations.
- `tool-boundary/raw/post-change-core-regressions.log`: approval and session regression suites.
- `tool-boundary/raw/post-change-scope-regressions.log`: persisted legacy/user permission checks.
- The recorded patch includes `permission_judge.rs`, `permission_inspector.rs`, the central permission getter, and focused tests.
