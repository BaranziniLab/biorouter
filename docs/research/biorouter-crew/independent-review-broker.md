# Independent source review: Crew broker and lifecycle

Date: September 22, 2026. Reviewer: root Astra integration lane, reviewing the broker lane's implementation. This report does not independently approve root-authored server, agent, or UI integration. The broker lane separately reviewed those surfaces. All runtime checks and test code belong to Luna.

Scope: `crates/biorouter-crew/src/{broker,lib,main}.rs`, including framing, signed device challenges, kernel UID binding, enrollment, channel ownership, source ACL propagation, worker admission, journal replay, attachments, lifecycle and restart handling.

This is an evolving source review, not final acceptance. Final integrated checks, real three-user application evidence and the frozen revision recheck remain required.

| ID | Priority | Finding | Corrected source / required regression |
|---|---|---|---|
| B1 | P1 | Every restart generated another random socket path, invalidating saved desktop connections. | The runtime basename is now journaled and reused. Existing ownership, socket identity and live listeners are checked. Stop preserves the descriptor and never unlinks a replacement writer's socket. Linux CLI restart/reboot-directory-loss regression remains required. |
| B2 | P1 | Enrollment could treat an active UID as implicit additional-device authorization. A replacement Unix account could inherit old memberships after manager enrollment. | Invitations for active UIDs now require the exact `existing_principal_id`; acceptance revalidates it. Changed usernames require offboarding, and fresh enrollment receives a fresh principal UUID. The kernel cannot automatically detect UID reuse with an unchanged username; explicit manager intent remains a documented boundary. |
| B3 | P1 | The desktop allowed attachment-only posts but the broker required nonempty text, leaving a normal file-sharing flow broken. | Human messages permit an empty body only when validated attachments or references are present. Worker projections still require a body. Luna added focused empty/attachment cases. |
| B4 | P2 | The 16 MiB logical-state cap and duplicated idempotency results could exhaust storage much earlier than the message-count ceiling suggested. | The protocol now quantifies capacity and the actual preservation/new-workspace procedure. There is no claim of online compaction or archival freeing space. The measured 50-user workload is still required; this is a disclosed capacity limitation, not a performance acceptance result. |

Inspected invariants include nonces bound to the socket, device and UID; one-use challenges; enrollment public-key pinning; fresh signatures on mutations; durable idempotency results with payload conflicts refused; live worker membership/policy checks; restricted-source propagation; complete-record corruption refusal; torn-final-record quarantine; fsync before acknowledgement; and explicit local-storage/node restrictions.

Source review does not prove crash safety on every filesystem, deny all hostile code sharing the hosting UID, establish institution-specific MFA behavior, or validate HIPAA operations. Keep those limits and the acceptance ledger visible in the PR.

## Subsequent integration review

The root review also identified a remote invocation durability gap: fsyncing the receipt file did not persist its newly created directory entry before execution. The correction fsyncs the hierarchy before helper spawn; the separate Astra broker lane reread that change. Runtime fault evidence remains Luna's responsibility.

The root lane added per-kernel-UID connection admission (eight per UID, 256 total) after identifying that one account could consume all 64 original global slots. The broker lane independently reread the correction. Luna verified the ninth connection from one UID is refused while a different UID remains admitted, and that disconnect releases a slot in the local Linux fixture. These changes do not replace final app, load, or institutional acceptance evidence.

## Complexity refactor review

The September 22 refactor separates broker dispatch, operation handlers, journal replay and pinned runtime recovery without relaxing the existing baseline. The root lane separately reviewed the transport lane's remote helper extraction, and the broker lane independently reviewed the transport registry and worker changes: signer validation remains before the challenge, worker policy is rechecked after waiting for the transport and after its response, registry persistence precedes publication, and job receipt durability precedes child spawn. No concrete source regression was found. Luna must rerun the broker and transport checks against the refactored revision; earlier runtime observations are not automatically promoted.
