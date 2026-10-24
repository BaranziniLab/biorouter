# BioRouter v1.93.4 controlled security assessment plan

> **Scope update (2026-10-23):** The user has directed that this work be reported as a defensive software security audit focused on identifying, fixing, and documenting product weaknesses. See [`AUDIT-INTENT.md`](AUDIT-INTENT.md), which supersedes this document's prospective attack-campaign language for ongoing work and manuscript reporting. This file remains unchanged as the dated record of the initial plan; existing observations are treated as bounded internal control and regression evidence.

Status: frozen before attack execution, 2026-10-22 (America/Los_Angeles).
Purpose: address BIOINF-2026-2449, Reviewer 2 major comment 7 with a small,
reproducible penetration and adversarial control assessment. This is an internal,
scoped assessment, not independent certification or a HIPAA compliance audit.

## 1. Frozen system and assumptions

- Current source: `f73ebf74c578717a7af64b221e57358a63dda8e3` (origin/main).
- Current released software: v1.93.4, source tag commit
  `b824e2c3e802e243804a150aa7c349fa6ce6a8b6`.
- No runtime Rust/Electron source differences exist between the release tag and
  the assessment source. The intervening changes are website/readme and a Windows
  packaging-check script. Save the actual diff as provenance.
- Black-box target: official `biorouter-cli_1.93.4_amd64.deb`, expected SHA-256
  `8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65`.
- Legitimate researchers are trained, cooperative, aware of HIPAA/PHI handling,
  and have agreed to follow institutional and software instructions. No PHI,
  real credentials, production accounts, or institutional services are used.
- Attackers: unauthenticated remote/browser client; separate unprivileged local
  account; malicious imported text or tool output acting through a misled agent.
- Excluded: root/administrator/physical compromise; malicious cooperating account
  owner; third-party extension implementation defects; resource exhaustion;
  clinical correctness; legal compliance. Same-UID access explicitly documented
  by the product is reported as a deployment limit, not a newly found exploit.

## 2. Controlled setting

Use disposable Docker containers with synthetic home/config/workspace directories,
no host home/credential mounts, no Docker socket mounted in a target, no published
ports, no privileged containers, dropped capabilities, and no-new-privileges.
Use a dedicated internal-only Docker network only if separate attacker/target
containers are required; otherwise use `--network none` and container loopback.
The target runs under a non-root UID; a distinct attacker UID tests local access.
Filesystem fixtures contain clearly synthetic canaries and never patient-like data.
Source and binaries are mounted read-only where feasible; writable scratch and
evidence locations are explicit. Setup/download connectivity is separate from
attack execution. Bound every request, command, and test duration and cap local
CPU workers. Stop only test-owned processes and remove test-owned containers.

Verify isolation using UID/capability/namespace/mount inspection and a benign
positive control before recording security results. Record the actual setting,
including any necessary deviation (for example a native component build).

## 3. Prespecified attack families and paired controls

| Family | Adversarial cases | Legitimate controls | Observable decision |
| --- | --- | --- | --- |
| A. API authentication | Missing, wrong and empty credentials; read/write/config/session requests; route-exemption lookalikes; forged forwarding headers | Health check; valid credential read and harmless synthetic write | HTTP response plus persisted state/canary check |
| B. Browser and WebSocket boundaries | Wrong/missing token; hostile, null, file and cross-port origins; wrong Host; unknown bridge grant; shell/token leakage and unsafe no-token exposure | Same-origin authenticated request/socket; valid browser navigation | Handshake status, response/body markers, state changes |
| C. Local user and filesystem | Separate UID reads of synthetic config/secrets/history; parent/absolute traversal; symlink and dangling-symlink escape | Owner access; in-workspace read/write/list | Actual bytes read/written and OS denial |
| D. Approval and policy enforcement | Malicious/changed tool arguments; hostile judge output; cache scope; managed-deny precedence; malicious imported text/role markers | Benign tool classification and permitted actions | Actual permission result and captured judge input, not model claims |
| E. Sandbox execution | Workspace escape, host-canary access, network attempt, readonly-mount writes, process timeout cleanup | Allowed workspace operations; non-root execution | Side-effect markers, process/container lifetime, exit status |
| F. Secret/provider boundaries | Synthetic credential arguments/output; cross-origin redirect leakage; privacy-mode boundary crossing where harness available | Benign text, same-origin requests and approved private-provider path | Captured synthetic requests/output and guard result |

Finalize the concrete machine-readable cases within these families before the
first measured run of each lane. Preserve each case manifest and its hash. New
cases arising from a result are explicitly exploratory and never silently added
to the prespecified denominator. Existing regression tests are a separate evidence
stream and are not relabeled as independent penetration attempts.

## 4. Execution and outcomes

For deterministic black-box cases, use three fresh daemon/container repetitions
when feasible. Repeat confirmed findings in an independently reset fixture.
Do not pool repetitions as independent vulnerabilities or model trials. Keep
paired valid-use controls, even if an attack fails. Component suites may execute
once with targeted independent confirmation of a finding.

Allowed outcomes: `blocked`, `attack_succeeded`, `allowed_control`,
`blocked_control`, `inconclusive`, `not_exercised`, `error`.
An expected rejection alone is insufficient if the protected operation was not
reached; inspect an effect marker or response that establishes the boundary.
Timeouts, startup failures, and skipped/unavailable environments stay in the data
and are never removed from the denominator without explicit accounting.

Raw records include run and case ID, family, case type, repetition, version and
commit, configuration, actor, attack input/fixture hash, expected protected effect,
observed status/effect, outcome, duration, evidence path and limitation. Preserve
commands, sanitized stdout/stderr, dependency/container versions, package hashes,
case manifest hashes and environment information. Never capture host environment
values, real tokens, or credential-store contents.

## 5. Quantitative reporting

Report unique cases and observations separately, by family and configuration:
attack successes / exercised attacks; blocked attacks / exercised attacks;
allowed controls / exercised controls; false blocks / exercised controls; and all
inconclusive/not-exercised/error counts. Also report confirmed unique findings.
Do not collapse components, default modes and optional modes into one score.
This is a purposive finite corpus: proportions describe these cases, not the
probability of compromise in deployment. Repeated deterministic cases establish
reproducibility; they do not create independent statistical samples. No security
guarantee follows from zero observed successful attacks.

A deterministic scripted provider may force malicious tool calls to test actual
enforcement. Such tests do not estimate stochastic prompt-injection resistance or
the accuracy of the Smart Approve model. Real-model trials, if unavailable in this
isolated setting, are explicitly unassessed rather than substituted with regex
matches. Native macOS Keychain/TCC, GUI/browser-engine behavior, and other platforms
are unassessed unless actually exercised and separately recorded.

## 6. Findings, recommendations and manuscript outputs

Do not change product behavior during the baseline assessment. Reproduce and
independently review candidate findings; distinguish documented limits, expected
behavior, test failures and actual defects. Search existing GitHub issues to avoid
duplicates. The user explicitly requested GitHub issues and recommendations; this
authorization governs disclosure despite the repository's usual private-report
policy. Public issue content uses synthetic fixtures only.

Every confirmed issue includes affected version/configuration, attacker and
prerequisites, observed and expected behavior, reproducible synthetic evidence,
practical impact/limitations, and concrete recommendations that retain ordinary
workflow convenience. Prefer automatic scoped authority, safe defaults and
transparent enforcement over repeated confirmations or disabling useful features.
Implementation and prioritization remain the user's decision.

Deliver: this plan; reproducible harness/case corpus; raw JSONL and CSV; environment
and provenance manifest; per-family summary and issue index; manuscript Methods,
Results and Limitations draft; and a response to Reviewer 2 comment 7. Cite only
observed results, report unresolved findings, and avoid certification claims.

## Method references

- OWASP Web Security Testing Guide v4.2:
  https://owasp.org/projects/web-security-testing-guide
- OWASP GenAI LLM01:2025 Prompt Injection (untrusted content, constrained authority,
  and adversarial testing):
  https://genai.owasp.org/llmrisk/llm01-prompt-injection/

The assessment selects applicable ideas from these guides; it does not claim
complete WSTG/ASVS/LLM Top 10 coverage.
