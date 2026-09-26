# Queued source ACL observation acceptance, 2026-09-23

This report records one bounded live acceptance run against the disposable AWS
Crew broker. It used fresh Alice and Bob local profiles and fresh device
preparations. The setup used the supported CLI/API flow for daemon startup,
device preparation, connection save/update, bridge connect, workspace
bootstrap, enrollment invite/accept, team/channel creation, channel invites,
task/run creation, and message history. No credential, registry, journal, or
runtime file was edited by hand. The local profiles explicitly selected
`BIOROUTER_DISABLE_KEYRING=true`, so this is real broker/SSH signing evidence
using the development plaintext credential backend; it is not encrypted-vault
qualification. The supported vault initializer correctly refused that backend.

The broker workspace was `e5a29121-4e8a-4e8a-8f5c-837eb9bd6b80`. Source A was
`273b0868-1256-4b5f-a32a-2a6d7e7be6a5`; destination B was
`d17a7109-5e41-406e-8069-9acb8f63b1cd`. Bob's principal was
`5036f5ad-4e66-4a87-a4b8-c63c4b5f9faf`. The derived progress message was
`0c79f306-b214-45eb-95dc-2f269582bff2`, from run
`3782a475-efa9-42b9-8525-3b3335043fd0`; its `source_channels` contained both
source A and destination B. The valid destination anchor cursor was
`54aa9898-2bbc-4392-b4fb-93df7115ad24`, and the derived frame cursor was
`0c79f306-b214-45eb-95dc-2f269582bff2`.

The exact focused command was:

```text
cargo test -p biorouter-server --lib routes::crew_observation::live_acceptance::real_source_acl_revocation_clears_enqueued_and_waiting_observer_frames -- --ignored --nocapture
```

It selected 1 ignored test, with 761 library tests filtered out. The test
passed in 1.09 seconds (cargo session 41764). The harness now calls the real
`CrewManager::connect` before the pre-revocation snapshot. Alice revocation is
performed by the supported child CLI command `crew --connection ...
--no-start --approval-key-stdin remove-member SOURCE BOB_PRINCIPAL`.

The passing assertions covered a real pre-revocation snapshot, one frame
queued in `mpsc(1)`, a second send observed pending on the full queue, and
source-A revocation. The queued frame was reauthorized before delivery; the
pending frame was withheld and drained after the terminal decision. The
terminal result was asserted to be either `policy_changed` or `stale_cursor`,
with `clear: true`; no bytes from canary `b31210b8` appeared in the terminal
frame, the next receiver read returned EOF, and the waiting frame was drained.
When the terminal code is `policy_changed`, the test checks non-null policy
epochs before and after revocation and requires them to differ; it then
confirms destination B remains usable. The `stale_cursor` branch does not
claim an epoch transition.
The executed test did not print a sanitized terminal-code or numeric-epoch
receipt, so this report retains the exact accepted error-code set and the
epoch-change assertion rather than claiming a selected branch or numeric
before/after values.

Sanitized evidence hashes:

- Derived frame JSON: `aab5cac93e7bce26dfe1aab3aa4e4989a4f766693dfbe03c9de757f7a19cf462`.
- Current CLI binary: `278db11f673b13ec866d9209a349610e1deda8f7800dc3eec36958b9b615ad3f`.
- Current daemon binary: `513c1d93052196938e1b35d20a9f5a0324532d3dd02d1f02e3de211023ed81c0`.
- Renamed live test source used by the passing run: `5fc7e508010bd5d2fa2c912abddfe73986fa2df6bd06f50bc8811ab88ae7950b`.

The harness was renamed to `crew_observation_live_acceptance_tests.rs` and its
test-only path was updated in `crew_observation.rs`, so privacy scanning does
not classify its synthetic/live acceptance code as production. A later
compile-only check of `biorouter-server --lib` passed after that rename.

This evidence proves the real broker, SSH bridge, signatures, source ACL
revocation, observer queue blocking, terminal clear/error, and EOF behavior for
this fixture, and exercises the conditional epoch-change assertion when the
terminal code is `policy_changed`. It does not establish the HTTP
authentication layer or encrypted-vault behavior. It also cannot recall bytes
already delivered to a receiver before revocation.
