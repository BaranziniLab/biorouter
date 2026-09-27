# Adversarial broker validation

The focused Luna test artifact adds four tests on product base `26f2a496`:

- `membership_revocation_between_blob_chunks_denies_completion_and_reads`
- `non_owner_cannot_archive_or_transfer_channel`
- `broker_open_rejects_concurrent_writer_and_allows_reopen_after_release`
- `request_version_id_and_parameter_bounds_reject_malformed_inputs`

Command:

```text
CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target cargo test -p biorouter-crew --test adversarial_contract -- --test-threads=1
```

Result: **4 passed, 0 failed**. The artifact SHA-256 is `89e34a3fd99a8a73efa53ab3881fa6f4805ef82ae1b2ba551c4393bb75245263`.

That hash is the initial historical artifact. The current shared-source rerun is also **4 passed, 0 failed** after workspace formatting and Clippy cleanup. Current hashes are `broker.rs` `995dbf1d9b377b57060010366fa899d1e6c3754a97311eeeb3e5faab2679ada4` and `adversarial_contract.rs` `a0f9712aa3b08ac59531507f49fc69187548166f06c41ffd4b4fffdd9add27bb`.

The parameter-bound test exercises typed `Broker::handle` requests. It does not establish malformed UTF-8 or transport wire-frame handling. Crash-durability at write/fsync boundaries remains open pending the separate Linux fault harness below.

## Linux journal fault harness

The ignored test `journal_fault_preserves_prior_ack_after_restart` in [`journal_fault_contract.rs`](../../../crates/biorouter-crew/tests/journal_fault_contract.rs) uses [`journal_fault_interposer.c`](smoke/journal_fault_interposer.c) under a disposable Linux Docker fixture. The interposer matches only an open descriptor whose `/proc/self/fd` target ends in `journal.jsonl`, arms from a fixture marker, injects one `EIO`, and records one hit. It never uses a system-wide preload or the main lab state.

Both `CREW_FAULT_CALL=write` and `CREW_FAULT_CALL=fsync` reached the real journal descriptor exactly once with injected `EIO` and passed. A third `CREW_FAULT_CALL=write CREW_FAULT_ERRNO=ENOSPC` run also passed. The faulting operation is a `message.post` with no physical side effect; afterward, cached replay of the prior acknowledged message succeeds, a new real `blob.begin` is rejected with `storage_failed` before creating a file, and the prior message's exact ID/body survives restart. A faulting blob operation with an uncertain fsync outcome can leave a referenced file that recovery may need; this harness does not assert unsafe orphan deletion or a grace-sweep policy. These are injected errno scenarios, not literal disk-full or power-loss evidence.

Exact scenario command (vary `CREW_FAULT_CALL` between `write` and `fsync`, and set `CREW_FAULT_ERRNO=ENOSPC` only for the third run):

```text
docker run --rm -v /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter:/workspace -v /private/tmp/crew-linux-target:/tmp/crew-target rust:latest bash -lc 'export PATH=/usr/local/cargo/bin:$PATH; cd /workspace; gcc -shared -fPIC -O2 -Wall -Wextra -o /tmp/crew_journal_fault.so docs/research/biorouter-crew/smoke/journal_fault_interposer.c -ldl; CREW_FAULT_CALL=write CREW_FAULT_ERRNO=ENOSPC CREW_FAULT_MARKER=/tmp/crew-fault-marker CREW_FAULT_HIT_FILE=/tmp/crew-fault-hits CREW_FAULT_SUFFIX=journal.jsonl LD_PRELOAD=/tmp/crew_journal_fault.so CARGO_TARGET_DIR=/tmp/crew-target cargo test -p biorouter-crew --test journal_fault_contract -- --ignored --exact journal_fault_preserves_prior_ack_after_restart --test-threads=1'
```

## Current CLI wire framing validation

The current Linux CLI artifact `/private/tmp/crew-linux-target/debug/biorouter-crew` (SHA-256 `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`) passed [`crew_wire_framing.py`](smoke/crew_wire_framing.py) in a fresh `rust:latest` Docker container. The fixture created a synthetic root-owned machine identity, ran the broker as ordinary UID 1000, connected through the actual Unix socket and kernel peer-credential path, and removed only its recorded runtime socket/state.

The four bounded cases were `invalid_utf8`, `oversized_frame`, `invalid_version`, and `truncated_client_close`. Invalid UTF-8, oversized, and truncated frames caused connection closure; invalid protocol version returned an `invalid_request` error response. Each case left `journal.jsonl` byte-for-byte unchanged, and a healthy `hello` succeeded before and after every case. The script reported `journal_unchanged: true`, `kernel_uid_socket: true`, and `result: pass`. This is a focused wire contract check, not fuzzing or a broader load result.

Reproduction:

```text
docker run --rm -v /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter:/workspace -v /private/tmp/crew-linux-target:/tmp/crew-target rust:latest bash -lc 'set -eu; printf 0123456789abcdef0123456789abcdef > /etc/machine-id; chmod 444 /etc/machine-id; useradd -u 1000 -m -s /bin/bash crew; runuser -u crew -- python3 /workspace/docs/research/biorouter-crew/smoke/crew_wire_framing.py /tmp/crew-target/debug/biorouter-crew'
```
