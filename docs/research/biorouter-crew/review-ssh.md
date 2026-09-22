# Independent SSH and integration review

Reviewed `implementation-plan.md` against the baseline source investigation, institutional probe script/results and AWS identity report. No remote actions or implementation changes were made. Line references refer to the plan as reviewed on 2026-09-21 Pacific.

## Findings

1. **P2 — Apply the mandatory SSH settings and reauthentication policy to every hop, not only the destination.** The example at `implementation-plan.md:101-118` sets `StrictHostKeyChecking`, `ForwardAgent` and `ForwardX11` only under `Host crew-service`; the two gates may inherit contradictory user configuration. Similarly, the app-owned socket and authentication-age rule at lines 134-136 does not explicitly exclude an inherited ControlPath/ControlMaster on a jump host. Reopening the final transport can reuse an older authenticated bastion master, so it does not establish renewed MFA at every required gate. Correct the example with a preceding common stanza covering all three selected aliases, and explicitly require app-controlled effective settings per hop, no unapproved inherited master reuse, and teardown/reauthentication of every policy-expired hop. Preserve user-selected credential and routing settings. Add the narrow fixture: final host reconnect while a preexisting jump master remains alive must not evade that jump's configured maximum authentication age. OpenSSH explicitly documents that destination options are not generally applied to jump hosts. [OpenSSH configuration manual](https://man.openbsd.org/ssh_config#ProxyJump)

2. **P2 — Narrow the institutional login evidence to the authentication actually measured.** The acceptance row at `implementation-plan.md:144` calls the result “Direct known-host key login.” `smoke/run_host_probes.py:13-18,28-29` supplies `BatchMode=yes` and `StrictHostKeyChecking=yes`, but it does not disable inherited ProxyJump/ProxyCommand or ControlPath reuse and does not record the negotiated authentication method. `institutional-host-smoke.json` proves successful framed/byte transport to each supplied endpoint under the current SSH profile, not necessarily a new direct public-key authentication. Rename the scenario/result to “Existing known-host SSH profile; noninteractive access and clean stdio” unless separate recorded evidence proves direct topology and key authentication. No new institutional login is necessary to fix this wording. The AWS report already correctly distinguishes its simulated topology, shared test key and untested MFA.

3. **P3 — Fix the sidebar source pointer.** `implementation-plan.md:36` names `ui/desktop/src/components/AppSidebar.tsx`, which is not the current implementation. The verified file is `ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx:101-144`. The Crew UI route seam at `ui/desktop/src/App.tsx:710-755` and the required human-only bypass of provider selection are accurately described.

## Reviewed without additional findings

The plan correctly separates the existing global external HTTP backend from new SSH workspace connections; does not treat the shared daemon secret, ACP or an SSH tunnel as multi-user identity; retains a kernel-authenticated Unix bridge; distinguishes local paths from remote attachments; and describes the actual existing non-image browser-upload gap. Authentication interaction is separated from protocol stdout, Windows/multiplexing compatibility remains unproven, and the plan explicitly marks interactive MFA and real institutional multihop validation as pending. The JSONL/journal design is consistent with the user's storage preference and does not silently rely on SQLite, Redis or a container platform. The existing built-in public internet tunnel is not being reused for Crew.

The main report's temporary missing feasibility link and AWS report filename are already being corrected by the parent task and are not separate findings here.

## Resolution verification

All three findings are addressed in the revised main plan, verified by rereading the actual file:

- Finding 1: `implementation-plan.md:102-107` now applies host verification, forwarding restrictions and disabled inherited multiplexing to every example alias. Line 139 explicitly requires app-owned per-hop sockets, rejects preexisting bastion-master reuse, and expires affected jump connections plus their dependents. This resolves the design omission; implementation and MFA tests remain required.
- Finding 2: `implementation-plan.md:147` now reports existing noninteractive known-host access and explicitly says the authentication method and fresh-MFA assurance were not established.
- Finding 3: `implementation-plan.md:36` now points to `ui/desktop/src/components/BioRouterSidebar/AppSidebar.tsx`.

No unresolved actionable findings remain from this SSH/source integration review. This resolution is a document review, not evidence of implemented SSH behavior or additional remote testing.
