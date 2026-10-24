# Independent methodology review and supplement specification

Review date: 2026-10-22 (America/Los_Angeles). This document reviews the frozen
assessment plan and defines reporting requirements. It is not a results report.
No product changes or attacks were performed to prepare this review or the
associated imported-content corpora.

## Frozen plan and adequacy

The reviewed `PLAN.md` has SHA-256
`75629068ee43da677e60a645d3e88876c5623a8d9cf89ddc574d9ff82c67ed14`.
The plan is adequate for an **internal, scoped gray-box security assessment**
addressing the reviewer's missing-evaluation concern. It does not establish
complete penetration-test coverage, independent auditing, certification, or HIPAA
compliance. Its strengths are a frozen target and threat model, synthetic fixtures,
paired legitimate controls, effect-based observations, preservation of errors and
unexercised cases, and separation of component, black-box, and model evidence.

Cooperative, trained users are a deployment assumption. They do not remove the
malicious-document/tool-output threat and are not a measured technical control.
An assertion inside external content that a user has approved an action is not
evidence of approval. Manual-review tests with scripted denials evaluate the
approval mechanism, not users' ability to recognize every deceptive request.

Prespecification remains lane-specific: each concrete manifest and its hash must
precede the first measured run of that lane. A frozen broad plan alone does not
make subsequently invented cases prespecified. Preserve exploratory additions
and independent confirmations under separate labels. The review does not amend
the frozen plan.

## Evidence classes that must remain separate

| Evidence class | What it can establish | What it cannot establish |
| --- | --- | --- |
| Released-binary request and state/effect probes | Behavior of the exercised routes and deployment settings | Complete API coverage or behavior of other platforms/modes |
| Production component calls | Behavior of the real component under explicit fixture prerequisites | End-to-end external attacker reachability without a demonstrated call path |
| Deterministic scripted provider/judge | Enforcement when supplied a particular tool call or judge response | Real-model injection resistance or model/judge classification accuracy |
| Production regex/guardrail detector | Flagged/unflagged strings and detector false flags within the finite labeled corpus | Prevented attacks, blocked legitimate workflows, Smart Approve accuracy, or stochastic model resistance |
| Real model plus genuine lower-trust content, if executed | Observed model/tool/approval behavior for the recorded model and setting | Protection against an unrestricted future attack population |
| Existing regression suites | Regression behavior for their own enumerated assertions | Independent penetration attempts or new adversarial observations |

The malicious-content corpus contains 20 distinct strings and the benign-control
corpus contains 10 legitimate inputs. Both manifests explicitly record that they
are planned, not results. A later detector run should preserve the manifest's
original bytes, hash its rendered inputs, and write observations to a different
file. It must use `flagged`/`unflagged` or equivalent detector-specific fields.
Do not force detector observations into `blocked`/`attack_succeeded` outcomes.

The benign corpus intentionally contains security-training quotations, role
delimiter examples, and unsafe-path documentation. A detector false flag is
defined against these stated legitimate tasks and labels; it is not an observed
false denial unless a complete task was actually blocked. Some malicious cases
target output integrity or persistent state, so treating every endpoint as
command-execution prevention would also be incorrect.

## Measurement requirements

1. **Reach and effect.** Confirm that the request reached its intended protected
   boundary. A startup failure, an unparsed fixture, or a source the model never
   consumed is unexercised or inconclusive. Observe returned bytes, file hashes,
   created markers, executed commands, sink requests, or artifact contents rather
   than relying on a model's refusal message.
2. **Authority.** For every finding, specify which actor controls the payload,
   workspace entry, credential, origin, or tool argument. Show the bridge from
   that control to the vulnerable operation. An ordinary test process invoking a
   component is not an unauthenticated external attacker demonstration.
3. **Configuration.** Retain actual UID, capabilities, mounts, namespaces, network,
   binary/source provenance, provider, policy, and approval mode for each lane.
   An intentionally allowed provider destination is not attacker exfiltration;
   predeclare allowed destinations. File access still requires the host process's
   OS permissions even when an application-level jail is bypassed.
4. **Denominators.** Report unique attack cases, legitimate controls, and repeated
   observations separately, per family and configuration. Include all errors,
   inconclusive cases, and unexercised endpoints. Report both unique findings and
   affected surfaces so a common root cause is not counted as multiple independent
   vulnerabilities.
5. **Uncertainty.** This is a purposive finite corpus. Proportions describe this
   corpus, not deployment compromise probabilities. Three deterministic repeats
   provide reproducibility evidence, not three independent vulnerabilities.
   No security guarantee or representative population confidence bound follows
   from zero successes. Real-model repeats, if performed, must preserve case
   clustering and expose variation rather than being pooled as independent attacks.
6. **Remediation.** Keep `identified`, `documented`, `fixed`, and `retested` as
   separate status fields. An issue, recommendation, or manuscript disclosure does
   not repair product behavior. Preserve the baseline results even if a future
   fix is evaluated separately. No automatic product fixes are authorized by this
   report preparation.

## Initial candidate finding review: dangling-symlink file writes

The first raw file-API log and the observational test fixture support a
production-component file-jail bypass: a dangling workspace symlink to a
not-yet-created outside destination is skipped by the resolver's
`Path::exists()` check; the later host-side write follows the unresolved symlink.
The same resolver is used by both backend file APIs. This is **not a Docker
container/namespace escape**; the Docker backend's file operation runs on the
host side.

Source reviewed: `crates/biorouter-sandbox/src/lib.rs` (workspace resolver and
`canonicalize_existing_ancestor`), `local.rs` and `docker.rs` (file writes),
`crates/biorouter-mcp/src/developer/jail.rs`,
`crates/biorouter-mcp/src/files_server/mod.rs`, and
`crates/biorouter-server/src/routes/apps.rs::inject_workspace_capabilities`.
The app path injects a writable FilesServer for a granted `files` capability;
its write method uses the shared resolver and then writes the candidate path.
This is a source-backed app call path, not yet an end-to-end runtime reproduction
at the time of this review. ComputeServer calls the sandbox's `exec` method, so
the leaf `SandboxClient::write` probe must not be described as a demonstrated
compute-tool exploit.

The demonstrated prerequisites are control of a symlink inside the workspace,
an induced/available write operation to that entry, an absent target, and host
process write permission at the destination. A different UID without workspace
write access cannot exploit the observed fixture merely by being logged in.
Imported-content reachability requires a demonstrated route for planting or
selecting the symlink and inducing the tool write. Do not call the current
same-process fixture an unauthenticated remote or separate-UID exploit. Final
severity and issue wording should reflect the strongest independently confirmed
production route, with the shared root cause counted once.

## Journal-neutral supplementary package

The final supplement should contain the following numbered items, with measured
results inserted from the final evidence inventory rather than draft placeholders:

1. **Supplementary Methods: threat model and deployment.** State cooperative-user
   assumptions; the three attacker classes; exclusions; released version/commit;
   container and native environments; credentials/PHI exclusions; approvals and
   allowed network destinations. Identify which environments were actually tested.
2. **Supplementary Methods: test selection and execution.** Describe the frozen
   plan and per-lane manifests, positive controls, repetition/reset procedure,
   time limits, effect checkers, and evidence classes above. Explain that the
   corpus is purposive and whether it received detector-only or real-model testing.
3. **Supplementary Table S1: complete case inventory.** One row per unique case
   and configuration, with actor, entry point, prerequisite, objective, expected
   effect, control pairing, applicability, evidence class, and raw-record reference.
4. **Supplementary Table S2: measured outcomes.** Per-family unique cases and
   observations, attack effects, legitimate-control outcomes, errors and unexercised
   cases. Add a separately labeled detector table for malicious flags and benign
   false flags. Do not combine these into a single security score.
5. **Supplementary Table S3: findings and disposition.** One root-cause finding per
   row, affected surfaces, actual attacker prerequisites, impact and limits,
   independent confirmation, issue/reference, recommendation, and the four separate
   identified/documented/fixed/retested statuses. Explicitly state unresolved defects.
6. **Supplementary Results narrative.** Summarize only observed behavior and cite
   tables/fixtures. State whether failures were reproduced and which tests could
   not be exercised. Recommendations are not experimental improvements unless
   a distinct changed build was actually retested.
7. **Supplementary Limitations.** Internal assessors; finite corpus; fixed software
   and configuration; native/macOS/GUI limitations; untested third-party extensions;
   lack of real-model data if applicable; no multi-institution replication; no
   clinical/privacy/legal certification. Explain residual risk from regex filters,
   model judgment, broad grants, and supported same-UID operation where relevant.
8. **Supplementary Data and reproducibility index.** Frozen plan/manifests and
   hashes, readme/environment manifest, commands, harness and changes, sanitized
   JSONL/CSV/logs, software/container versions, per-case outcomes, issue index,
   and checksum inventory. Ensure raw artifacts contain only synthetic values.

The main manuscript should point to this supplement and describe the observed
controls as tested under these conditions. Replace broad unqualified assurances
with the actual tested boundaries. A Reviewer 2 #7 response should state what was
newly evaluated, give the final case/control/outcome numbers with table references,
identify unresolved findings and remedial status, and state what was not measured.
It must not say all critical issues were fixed merely because they were identified
and addressed in the discussion. A Reviewer 1 #3 response can point to the explicit
limits of regex filtering and model-mediated approval and the separate empirical
evidence for deterministic enforcement.

## Methodological references

- [OWASP Web Security Testing Guide v4.2: introduction and objectives](https://wstg.owasp.org/v4.2/4-Web_Application_Security_Testing/00-Introduction_and_Objectives/)
  supports systematic, documented evaluation of application security controls.
- [OWASP LLM01:2025 Prompt Injection](https://genai.owasp.org/llmrisk/llm01-prompt-injection/)
  distinguishes lower-trust content injection and recommends testing trust boundaries.
- [OWASP AI Agent Security Cheat Sheet](https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html)
  identifies tool misuse, approval bypass, persistence, exfiltration, and other
  agent-specific abuse cases and the configuration/evidence needed for validation.

These references inform selected methods. They do not establish full standards
coverage or endorse the assessment's findings.
