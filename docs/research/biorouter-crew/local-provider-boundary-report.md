# Local provider boundary evidence

This is API-harness evidence for the real Crew/provider path. It is separate from graphical G05 acceptance. Each run used the disposable `biorouter-crew-ssh-luna` Linux fixture, a fresh broker state, synthetic Alice user proof, and a loopback HTTP sink. No AWS source or desktop credentials were used.

The daemon artifact was compiled from the current product worktree. Its SHA-256 is the primary provenance key; the checkout contained uncommitted product changes, so the observed Git revision is context only and does not fully identify the daemon inputs.

Artifacts used for every scenario:

- daemon: `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd`, SHA-256 `1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86`
- disposable Linux broker source artifact: `/private/tmp/crew-linux-target/debug/biorouter-crew`, SHA-256 `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`
- current `crates/biorouter-crew/src/broker.rs` source fingerprint: SHA-256 `995dbf1d9b377b57060010366fa899d1e6c3754a97311eeeb3e5faab2679ada4`
- observed checkout revision: `431ec13472cb26b00dae7c5c3617c5ef95fa5618` (context only; uncommitted inputs were present)

The harness accepts `--broker-binary` explicitly, copies that artifact to an owned per-run path under Alice's disposable fixture home, verifies the copied hash, starts the broker as Alice, and removes only that exact path during cleanup. Every command below completed with `api_harness: pass`; the sink was checked for private-marker absence after each refusal.

## Public control and workspace-private refusal

Command:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario core
```

Result:

```json
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86","graphical_g05":"not_run","private_refusal":{"code":"crew_request_refused","error":"Crew broker refused request: {\"code\":\"privacy_denied\",\"message\":\"privacy_denied: Private workspace or connection\"}"},"public_run":{"status":"completed"},"sink_bytes":17462,"sink_count":1,"sink_sha256":"3a977dddbebd2a8f532ac5a1173c88a85026752ffb93ae2194c030b1ca8b7b2e","workspace_id":"78ac70c5-64b9-4532-a57c-b0a3a3822197"}
```

The public control request contained `LUNA_PUBLIC_SAFE_FIXTURE`. The later workspace-private run was refused and the sink count remained one.

## Personal-private mode after live public control

Command:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario personal-private
```

Result:

```json
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","control_sink_count":1,"daemon_sha256":"1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86","refusal":{"code":"crew_request_refused","error":"Private cluster blocks public models"},"scenario":"personal-private","sink_count":1,"workspace_id":"1582083e-f340-42ca-baf1-374a27f625cf"}
```

The public control run first reached the live sink. The harness then updated only the saved connection mode to private, reconnected, and attempted a public-model run; the specific private-cluster refusal arrived without increasing sink traffic.

## Verified private alias while original public connection remains active

Command:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario alias
```

Result:

```json
{"alias_refusal":{"code":"crew_request_refused","error":"Private cluster blocks public models"},"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86","scenario":"alias","sink_count":1,"workspace_id":"efa14980-394e-4334-98fe-e6bdb3f32ed6"}
```

The initial public control run produced one sink request. A second verified descriptor to the same broker/workspace was connected in private mode. The subsequent public attempt was made through the original public connection and was refused; no additional sink request appeared.

## Same public-safe channel after private retained message

Command:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario restricted-history
```

Result:

```json
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86","history_refusal":{"code":"crew_request_refused","error":"Crew broker refused request: {\"code\":\"privacy_denied\",\"message\":\"privacy_denied: retained context is restricted\"}"},"scenario":"restricted-history","sink_count":1,"workspace_id":"58978271-439e-4071-adf9-51963a6bb68e"}
```

The harness posted the public marker and completed a public control run on the original public-safe channel. It switched the workspace private, changed the connection to private, posted `LUNA_PRIVATE_CREW_SECRET` to that same channel, switched the workspace and connection back to public, and verified the channel classification remained `public_safe`. A new public run on that channel returned the specific retained-context denial; the sink count stayed one and no recorded request body contained the private marker.

These are API-harness results only. Graphical G05 remains `not_run`, and these cases do not establish MCP-grant or UI acceptance.

## Focused R9 worker-request race gate

The test-only additions in `crates/biorouter/src/crew/mod.rs` exercise the real `CrewManager::worker_request` path with a controlled fake SSH child. The two selected tests are:

- `crew::tests::worker_request_rechecks_policy_before_writing_transport`: a successful fake-SSH control request establishes one baseline line; the test then captures the shared transport `Arc` while the worker is queued behind its mutex, changes policy mode/epoch, and asserts the request count remains exactly one.
- `crew::tests::worker_request_reports_possible_effects_after_inflight_policy_change`: the fake SSH child holds a response, policy epoch changes during the request, and the worker refusal contains the possible-effects diagnostic.

Exact focused test command:

```text
source bin/activate-hermit && CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target CARGO_BUILD_JOBS=2 cargo test -p biorouter --lib worker_request_ -- --nocapture
```

Result: `2 passed; 0 failed; 4183 filtered out`.

Strict Clippy command:

```text
source bin/activate-hermit && CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target CARGO_BUILD_JOBS=2 cargo clippy -p biorouter --lib --tests --no-deps -- -D warnings
```

Result: passed. Repository `cargo fmt --all -- --check` also passed.
