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

## Private Ollama API acceptance (API evidence; graphical G05 separate)

One authorized run used the prepared `private-ollama` scenario with the same daemon and broker artifacts above. It used the isolated Alice workdir and synthetic CSV fixture, `OLLAMA_TIMEOUT=120`, and a 240-second overall poll bound.

Command:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario private-ollama
```

The actual run admitted and completed in approximately 75 seconds:

```json
{"run_id":"3d60d311-eb1a-427a-b1af-f3d31dea35f4","connection_id":"b80739b1-c407-499a-9a6c-d06909fc9592","channel_id":"aae76f45-3394-4039-850a-4679f256ae3e","session_id":"20260922_1","status":"completed","error":null}
```

The first run cannot be used to judge provider behavior. Its parser incorrectly assumed that `conversation` was an object containing `messages`; the API serializes the Rust `Conversation` tuple as a non-empty message array. The run JSON above is retained, but the session body was not persisted before the harness cleaned its disposable profile, so no raw tool trace can be recovered from that run. No inference retry was performed before correcting this provenance gap.

The harness now requires a non-empty array, an assistant `toolRequest` for the exact `request`/`crew__request` call with `method=remote.read` and `params.path=input.csv`, a matching `toolResponse` by request id, and parsed `remote.read` JSON containing the exact synthetic CSV. It separately checks assistant text for `rows=3` and `total=60`; prompt text cannot satisfy any of these checks. Offline parser regression:

```text
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/biorouter-crew/smoke/local_provider_boundary.py --parser-self-test
```

Result: `{"parser_self_test":"pass"}`. At that point, a corrected authorized replay was still required for actual provider evidence; the replay and retained-session validation are recorded below.

The corrected replay prompt asks the model to calculate `rows=N total=M` without supplying expected values and starts with `/no_think`. Its `--evidence-dir` option saves the terminal run record and a credential-redacted session response even when the run fails after exposing a session id.

### Authorized corrected replay

The one authorized replay used the same explicit artifacts (`biorouterd` SHA-256 `1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86`; `biorouter-crew` SHA-256 `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`) and the corrected calculated-output prompt:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario private-ollama --evidence-dir /private/tmp/biorouter-crew-private-ollama-evidence
```

The run completed with `error=null` (`run_id=d1f1b102-491d-4dc7-8081-fda624bc8cd3`, `session_id=20260922_1`). The harness initially exited failed closed because the real tool response text is wrapped in `<tool-output ...>...</tool-output>` around its JSON payload; it had already retained the sanitized session at `/private/tmp/biorouter-crew-private-ollama-evidence/private-ollama-session.json`.

Offline parsing of that retained response after the wrapper fix passed the typed evidence check: message count 4; assistant `crew__request` ID `call_cwp7nmni` with `method=remote.read`, `params.path=input.csv`; matching successful tool response with `size=27`, SHA-256 `1a0eaac8a6c35e5745cf174f9fbe8fce67a9deebaedc1eab732a1b2a7cdfb521`, and exact CSV rows A=10, B=20, C=30; assistant completion `rows=3 total=60`. This is API-harness evidence of a real private Crew tool round trip, with no retry. The harness and parser self-test now pass against the retained evidence; graphical acceptance remains separate.

## Carol GUI retained-session diagnostic (read-only)

The requested GUI run `9f1df8bb-2094-4a40-8e47-cdc3da43cd66` maps through `biorouter/state/crew/runs.json` to session `20260922_2`. The saved descriptor has remote root `/home/carol/work/crew-fixture`. All four typed tool calls used the same arguments (normalized argument SHA-256 `2c01a779652c4fe3fd919100a75a8296756df0d97f45ea5e163510f6cffae8d6`):

```json
{"connection_id":"55441429-e81c-4991-b1d4-283961df5e18","method":"remote.read","path":"/tmp/biorouter-crew-dev-carol-clean/biorouter/data/crew/tasks/crew-task.csv"}
```

Each request had a distinct call ID (`call_vypqe2uu`, `call_oaubpffc`, `call_z8ys9z1j`, `call_6xn5555n`) and a matched `toolResponse` with `isError=true`:

```text
[tool_error kind=tool_failure retryable=false] ... Connection is outside the approved run scope
```

The loop guard then warned after repeated identical non-retryable failures. The final assistant summary claimed a 10-row file and aggregate 42 without any successful tool response, so it is unbacked and must not count as acceptance. This is a model/path-following failure; no profile or session state was changed by the diagnostic.

Session `20260922_4` is a separate later UI run (`a8aa531d-a119-46eb-aefe-d2dd85e3128d` in the run registry). It used the correct relative request with no explicit connection ID:

```json
{"method":"remote.read","params":{"path":"crew-task.csv"}}
```

That typed request succeeded and returned `sample_id,value` with A=10, B=20, C=30, size 31, SHA-256 `b8d8853f57f79b6bc9e4c3736b9f7c840e0975ea0606d31cc934349d5606335a`. The session then ended with: `Ollama completed without answer text or tool calls`; there was no final assistant answer after the successful tool result. This separates a valid Crew tool round trip from the remaining empty-completion/model behavior.

### API-positive versus GUI session settings

The retained API-positive session and GUI session `20260922_4` both used provider `ollama`, model `qwen3:8b`, `toolshim=false`, and null `context_limit`, `temperature`, `max_tokens`, `toolshim_model`, and `fast_model`. Neither retained record exposes a stream setting or reasoning-effort override. Both prompts began with `/no_think`; the API prompt requested one `remote.read` on `input.csv` followed by `rows=N total=M`, while the GUI prompt requested `remote.read` followed by additional remote processing and attachment work.

The API-positive conversation had four messages: prompt, typed request, typed response, and final assistant `rows=3 total=60`. Its retained sanitized evidence does not preserve token counters or a provider finish reason. GUI session `20260922_4` had five messages and recorded 12,085 total tokens (7,212 input; 1,047 output in the session row); server logging records the turn ending `finish_reason=stop` after the successful tool response, with no final answer text or second tool call. Thus the observed difference is prompt/workflow completion after a successful tool call, not provider/model configuration; no inference was rerun.

## Prospective commit hygiene and local model metadata

Read-only prospective checks covered 18 changed or untracked paths. All 3 Python files parsed with `ast.parse`, the changed JSON fixture parsed successfully, and all 10 Markdown files had balanced fences. No private-key body markers or known Carol device/node/workspace public identifiers were present; retained Ollama evidence remains outside the worktree under `/private/tmp`. These checks do not count additional runtime coverage.

All 52 relative Markdown links in the prospective reports resolved successfully. The parser self-test and retained-session positive extraction were rerun offline:

```text
PYTHONDONTWRITEBYTECODE=1 python3 docs/research/biorouter-crew/smoke/local_provider_boundary.py --parser-self-test
{"parser_self_test": "pass"}

retained_session_positive pass messages 4 request_id call_cwp7nmni
```

The exact read-only Ollama checks were:

```text
GET http://127.0.0.1:11434/api/ps
POST http://127.0.0.1:11434/api/show {"name":"qwen3:8b"}
```

`/api/ps` reported zero currently loaded models, so active runtime `num_ctx` was not observable. `/api/show` reported family `qwen3`, parameter size `8.2B`, quantization `Q4_K_M`, and the model-advertised context length `40960`; this is an architecture/model metadata limit, not evidence of the active runtime context setting. No model call or configuration mutation was made.

## Final daemon synthetic boundary rerun

The four API-only scenarios were rerun sequentially against daemon `/private/tmp/biorouter-crew-target/debug/biorouterd` (SHA-256 `6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef`) and broker SHA-256 `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`. Each used a separate disposable fixture; the live three-user lab was not touched.

| Scenario | Positive control | Negative result | Sink/canary result |
| --- | --- | --- | --- |
| `core` | Public run completed; sink count 1, 17,691 bytes | Private transition returned `privacy_denied: Private workspace or connection` | Sink count remained 1; no private marker reached the sink |
| `personal-private` | Public control sink count 1 | Mode-only private update returned `Private cluster blocks public models` | Sink count remained 1; no private marker reached the sink |
| `alias` | Original public connection control sink count 1 | Public attempt through the connected private alias returned `Private cluster blocks public models` | Sink count remained 1; no additional request or private marker |
| `restricted-history` | Public-safe channel control sink count 1 | Same channel after private retained message returned `privacy_denied: retained context is restricted` | Sink count remained 1; private marker absent from all sink bodies |

Exact commands used the same form, changing only `--scenario`:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /private/tmp/biorouter-crew-target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario core
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /private/tmp/biorouter-crew-target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario personal-private
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /private/tmp/biorouter-crew-target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario alias
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /private/tmp/biorouter-crew-target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario restricted-history
```

All four reported `api_harness: pass`; graphical G05 and real-model acceptance remain separate.

Recorded JSON results:

```json
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef","graphical_g05":"not_run","private_refusal":{"code":"crew_request_refused","error":"Crew broker refused request: {\"code\":\"privacy_denied\",\"message\":\"privacy_denied: Private workspace or connection\"}"},"public_run":{"status":"completed","error":null},"sink_bytes":17691,"sink_count":1,"sink_sha256":"33749617dfbcf0f66d8d430a379d66b9e709f5390a522cd3705bf714deefd5a"}
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef","control_sink_count":1,"refusal":{"code":"crew_request_refused","error":"Private cluster blocks public models"},"scenario":"personal-private","sink_count":1}
{"alias_refusal":{"code":"crew_request_refused","error":"Private cluster blocks public models"},"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef","scenario":"alias","sink_count":1}
{"api_harness":"pass","broker_sha256":"7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759","daemon_sha256":"6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef","history_refusal":{"code":"crew_request_refused","error":"Crew broker refused request: {\"code\":\"privacy_denied\",\"message\":\"privacy_denied: retained context is restricted\"}"},"scenario":"restricted-history","sink_count":1}
```

## Carol fresh-control corroboration

Read-only inspection of the clean Carol profile after the source `373acdf5` / daemon `6c19b87f...` UI control mapped completed run `fefe37f6-1ed6-4034-9e77-e6cde1056857` to session `20260922_5` (`status=completed`, `error=null`). The typed request ID was `call_q3k44r7t`:

```json
{"name":"crew__request","arguments":{"method":"remote.read","params":{"path":"crew-task.csv"}}}
```

The matched typed response had `isError=false` and returned the synthetic 31-byte CSV (`sample_id,value`, rows A=10, B=20, C=30) with SHA-256 `b8d8853f57f79b6bc9e4c3736b9f7c840e0975ea0606d31cc934349d5606335a`. The assistant completion quoted the exact returned data and metadata, including the 31-byte size and digest. This corroborates the actual app control path; it is separate from the API harness and does not replace graphical acceptance. A later run `f28385fb-3821-4b17-87c2-563ea06267d0` remained active for the separate processing/attachment task and was not inspected.

## Carol processing and attachment corroboration

The previously active processing run is now complete and is distinct from the control run: run `f28385fb-3821-4b17-87c2-563ea06267d0` maps to session `20260922_6` on channel `8adbc3a7-b9c1-4300-96bf-858270137631`, with `status=completed` and `error=null`.

The typed calls and matched successful responses were:

1. `call_oqi83bgv`: `remote.execute`, executable `python3`, one `-c` script, relative input path `crew-task.csv`, timeout 30 seconds, idempotency key `summarize-crew-task`. The script reads stdin, counts lines, then attempts to parse the same stdin with `csv.DictReader`, computes a numeric `value` sum, and writes `summary.csv`. It is not hard-coded to prior chat values, but it consumes stdin before parsing it, leaving no rows for the second read.
2. `call_jlhe2gaa`: `remote.job_status` for job `fa807bb05226f28b420aab09d9c161bb4e23c0f6bd98df406bfa88595e48611b`; response `status=completed`, `exit_code=0`, empty stderr, stdout `Row count: -1\nSum: 0\n`.
3. `call_f2u28h3v`: `remote.attach` for relative `summary.csv`; response `isError=false`, attachment identity `cd1ffa13-bc70-4052-8214-6d41d1e43f1a`, channel post ID `537ef76a-109f-481b-9bab-c3b71f5287b1`, body `Attached summary.csv`.

Read-only direct fixture verification in the local Carol SSH container found `/home/carol/work/crew-fixture/crew-task.csv` at 31 bytes with SHA-256 `b8d8853f57f79b6bc9e4c3736b9f7c840e0975ea0606d31cc934349d5606335a`, and generated `summary.csv` at 19 bytes with SHA-256 `746f256b10c3b912276f673e1bb7b60484582a1eaf4f70b76b3125b53fbc6fe0`, contents `Row count,-1` and `Sum,0`. The assistant’s claim that the attachment identity was a hash was inaccurate; the typed attach response provides an attachment ID, while the independently verified file digest is the SHA-256 above.

## Alice and Bob owned-task corroboration

Read-only inspection found distinct completed runs and sessions:

| Principal | Connection / SSH target | Run | Session | Status | Persisted assistant result |
| --- | --- | --- | --- | --- | --- |
| Alice | `fd27911e-23cd-4b35-97ad-9f0710917224` / `alice@127.0.0.1` | `1d7ca060-9dd4-44fc-8294-47ac2dd19f06` | `20260922_1` | completed, no error | `Alice has reviewed the channel history.` |
| Bob | `18f6d0e2-dc94-4aa6-89d9-22c7e14f8848` / `bob@127.0.0.1` | `a2090ea3-b7e6-4f76-b719-c57879820e32` | `20260922_1` | completed, no error | `Bob: Task completed.` |

Neither retained session contains a typed `messages.history`, `run.project`, or other Crew tool request/response; each has only the user prompt and one assistant text. The prompts explicitly asked the model to read an empty clean history and prohibited remote tools. Therefore these records corroborate distinct completed Alice/Bob tasks and principal labels, but do not establish an actual Crew history read or project-post tool call beyond the persisted model text.

## Final daemon rerun (source `aac5f4ac`, daemon artifact `1ce50cb3`)

The same four API-only scenarios were rerun sequentially against `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd`, SHA-256 `1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407`, with unchanged broker binary `/private/tmp/crew-linux-target/debug/biorouter-crew`, SHA-256 `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`. Each invocation used a fresh disposable API fixture; the live three-user profiles and GUI broker were not touched. The harness performed an allowed public control first, then asserted the private request was denied, sink count did not increase, and forbidden private markers were absent.

| Scenario | Public control | Private transition/result | Sink/canary assertion |
| --- | --- | --- | --- |
| `core` | Sink count 1, 18,926 bytes, SHA-256 `5b401dba02ec56d5bddee1d0c54b853fd0bc8191430d198a228d82fe12f337f0` | `privacy_denied: Private workspace or connection` | Count stayed 1; no private marker |
| `personal-private` | Sink count 1 | `Private cluster blocks public models` | Count stayed 1; no private marker |
| `alias` | Original public connection sink count 1 | Connected private alias returned `Private cluster blocks public models` | Count stayed 1; no additional request/private marker |
| `restricted-history` | Public-safe channel sink count 1 | `privacy_denied: retained context is restricted` | Count stayed 1; private retained marker absent |

Exact commands:

```text
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario core
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario personal-private
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario alias
PYTHONDONTWRITEBYTECODE=1 python3 -u docs/research/biorouter-crew/smoke/local_provider_boundary.py --daemon /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew --scenario restricted-history
```

All four commands returned `api_harness=pass`; `graphical_g05` was not run.

## Alice selected-source and marker corroboration

Read-only inspection of Alice’s later runs found two separate records. Run `818c4110-d768-4d2e-8c4c-9e0d2df6ecf7`, session `20260922_2`, issued typed `messages.history` request `call_dkg0pxjv` for channel `e7d5fa02-79cf-4b69-8953-d2f6625a8a92`, cursor `99`; the matched response advanced to cursor `138`. Every returned message had `source_channels` containing only that same channel. No Synthetic Lab / `#general` channel ID or cross-channel source appears in the request or response. The run’s assistant text claimed no additional context was found, but no typed post request is present.

Run `b460d8a7-b768-4482-b6e4-e3fb4c84933f`, session `20260922_3`, had no typed Crew request or post response. Its persisted broker run scope does include both source channel `55441429-e81c-4991-b1d4-283961df5e18` (`Synthetic Lab / #general`) and destination `8adbc3a7-b9c1-4300-96bf-858270137631` (`#fresh-control`). The injected history snapshot cursor `150` nevertheless contained only current-channel messages, including marker message `ba20d16e-70a4-4b9e-b3f1-482f849801b1` from `#fresh-control`; this is consistent with the current-channel history injection path and does not prove whether the selected source was read. The actual seeded human marker is broker message `7670c3c6-2c79-430c-ae57-5b8a4aa3ce3f`, authored by Carol (`fb4ec34a-c74c-4a87-8e34-2d3bbde459c7`) in `Synthetic Lab / #general`, sequence 141, body `Cross-channel synthetic fact: marker KAPPA-9413 belongs to Carol and is restricted QA data.` The assistant repeated a different CSV marker and said it would post, but no explicit typed post call or matching broker `message.post` entry with run ID `b460d8a7-b768-4482-b6e4-e3fb4c84933f` was observed; the journal records `run.project` and run revocation only for this run. These records prove scope admission included both IDs and identify the seeded marker’s true channel, but do not prove a successful cross-channel history read or post.

## Carol corrected processing attempt (completed, failed steps)

The next distinct Carol processing run is `f5d35bba-aaa9-4d28-9c25-7cd8d9ab5098`, session `20260922_7`, channel `8adbc3a7-b9c1-4300-96bf-858270137631`; the final read-only run ledger reports `status=completed`, `error=null`. There is no cancellation field or cancellation event in the persisted run ledger. Its typed trace shows:

- `call_lx135slz`: `remote.execute` with a `csv.DictReader` script, but the script used one-line `with` statements and returned a running job `f1e5a325b0065ef80cbca5c20414fa2ef2f584caf837dd0fa066a65a7f292b6b` without a later status record.
- `call_7mdaiq9z`: revised execute was refused as `remote_operation_denied: idempotency conflict`.
- `call_mngo9nr0`: unique retry job `60a07a44701eea14ebc93c3ff5ed50e4553f6c7cd1432b061347df30c7aec267`; its status call `call_iepgpq4q` returned `failed`, exit code 1, empty stdout, and Python `SyntaxError` at the one-line `with` statement.
- `call_folzq9ss`: triple-quoted retry was refused as an idempotency conflict.
- `call_1pix1n32`: unique job `229fd4d9a9f43cb6e9f9835625015b6455eef9ded2ed13c65fb8afcc0388591d`; status `call_v39l1hb7` returned `failed`, exit code 1, empty stdout, and the same one-line `with` `SyntaxError`.
- Further attempts reused `csv_sum_20260922_final` and were refused as idempotency conflicts. No attach call or successful output exists in this session.

The model’s scripts did attempt to read `crew-task.csv` and compute the `value` column; no hard-coded result was used. Direct read-only fixture verification still finds the prior malformed `summary.csv` (19 bytes, SHA-256 `746f256b10c3b912276f673e1bb7b60484582a1eaf4f70b76b3125b53fbc6fe0`, `Row count,-1` / `Sum,0`) from run `f28385fb...`; the corrected run has not changed it. Carol’s profile-local BioRouter `/api/ps` endpoint returned 401 during read-only inspection; this is distinct from Ollama and did not expose model metadata. A separate direct Ollama query at `127.0.0.1:11434/api/ps` was unavailable, so active runtime context remains unknown.

## Carol corrected processing retry (completed, failed before attach)

A subsequent distinct run `52d61c70-775a-484d-bfab-1f05b0623ddd` mapped to session `20260922_8` on the same channel and connection. The final run ledger reports `status=failed` with error `Task stopped; inspect its conversation for details`. Its only execute/status pair was:

- `call_peok02fy`: `remote.execute` returned job `f4314f2cbf4bc6eb127f633a6073399d5cf05963e906f08289de295dd88bb84d`, `status=running`, `exit_code=null`, empty stdout/stderr. The persisted argv was a generated `python3 -c` triple-quoted script rather than the requested prevalidated helper argv.
- `call_997zy8xo`: `remote.job_status` returned `status=failed`, `exit_code=1`, empty stdout, and `SyntaxError: unterminated triple-quoted string literal`; no `remote.attach` request or successful output exists.

The session then contains only an empty assistant message followed by the persisted “Ollama completed without answer text or tool calls” failure. This is a failed agent processing workflow: the remote tool calls returned concrete job and syntax-error responses, then the provider emitted an empty completion. It is not evidence of a computed `summary.csv`; the prior malformed `summary.csv` remains the only GUI output observed.

Read-only session metadata records provider `ollama`, model `qwen3:8b`, `context_limit=null`, `max_tokens=null`, `temperature=null`, and `toolshim=false`. Session 8 token ledger totals are input 475, output 866, total 17,252 (with cached input 15,911 on the final event). No provider finish reason or output completion field is retained; the persisted terminal reason is the empty-completion failure above.

## Carol fixture summarizer readiness

At the explicit test-only fixture-write request, I provisioned `/home/carol/work/crew-fixture/summarize_csv.py` in the existing local SSH container. The helper accepts the exact agent argv:

```text
["python3", "summarize_csv.py", "crew-task.csv", "summary.csv"]
```

It validates the required `sample_id` and `value` columns and row values, computes the count and integer sum from `csv.DictReader`, and writes canonical `row_count,total` CSV. The script was run locally and remotely as Carol against the real synthetic `crew-task.csv`; both computed `row_count=3 total=60`. Script SHA-256 is `901605725a653a05544cd83bb7af79f711321475fb6d812a91cb991cc7f544d6` locally and remotely. The separate remote verification output `fixture-check-summary.csv` is 21 bytes with SHA-256 `29f857b262032224f20e849e76bed6907e8e02846835a632c8380b2bafb91ab3` and contents:

```text
row_count,total
3,60
```

The existing malformed `summary.csv` was left untouched so the UI agent’s corrected task can produce and attach `summary.csv` itself.

## Carol processing-clean-2 backend corroboration (read-only)

The latest clean-channel Carol run is `4d7554d4-9bb7-460b-8766-f9904c96cb04`, session `20260922_9`, destination channel `c874aef4-10b5-41d3-99f7-6d6c90c4e644`, and granted connection `8c991d2f-afef-4a3c-9d52-36647b4f76c7`. The profile-local run ledger (`biorouter/state/crew/runs.json`) records `status=cancelled` and `error=null`; the persisted session ends with an inline `Stopped.` notification at 16:34:26 UTC. This is a read-only backend inspection; no profile, broker, or remote fixture state was changed.

The retained typed trace proves the following exact calls and outcomes (idempotency-key values are omitted here):

| Call | Typed request | Matched result |
| --- | --- | --- |
| `call_vcnd12a4` | `remote.execute`, argv `['python3', 'summarize_csv.py', 'crew-task.csv', 'summary.csv']` | Job `3dcb6078a21fc8f1d3cd017f6912c732b442dc3f4250ebe5561647ff7e041966` initially `running`; `call_6t1hl64y` later returned `failed`, exit 2, stderr `python3: can't open file '/home/carol/work/crew-fixture/summarize_csv.py': [Errno 1] Operation not permitted` |
| `call_4px7swxg` | `remote.list` with absolute `/home/carol/work/crew-fixture/` | Broker refusal: `invalid_params: path must remain relative to the approved work directory` |
| `call_sm03oe01` | `remote.list` with relative `.` | Success; listed `crew-task.csv`, `summarize_csv.py`, `fixture-check-summary.csv`, and `summary.csv` |
| `call_vcwah9jk` | `remote.execute` with the same exact argv but malformed `idemp3` field | Broker refusal: `invalid_params: idempotency_key must be a string`; no job was started |
| `call_hugg77re` | `remote.execute`, same exact argv | Job `27e92049269a5951eabd93f7d9dcfcf01ba5ee24fecaf81fb92529294f1f3e8a` initially `running`; `call_qsc74b0n` returned `failed`, exit 2, with the same `Operation not permitted` helper-path error |
| `call_vu0i2t3f` | `remote.execute`, same exact argv | Job `cf8635d5cce88020688560e822b016826c09bddf87da037f2f74f5527acdc6e6` returned `running` with empty stdout/stderr; no terminal response was persisted before stop |
| `call_4myj7eql` | `remote.execute`, same exact argv | Job `619c7c696d295c0b8b16cebf34fc8af0ab6db4f683c29714267278492d996aee` returned `running` with empty stdout/stderr; no terminal response was persisted before stop |
| `call_xctos830` | `remote.execute`, same exact argv | Job `d15bf963b452c519511342807382aa13e325a266ed2702400be95f044c1bf103` returned `running` with empty stdout/stderr; no terminal response was persisted before stop |

There is no typed `remote.attach` request in session `20260922_9`, no successful output, and no generated `summary.csv` result attributable to this run. The concrete evidence classifies this as a cancelled processing workflow with two terminal remote path failures, one parameter validation refusal, and three in-flight jobs left without persisted terminal responses. It must not be reported as a helper success or as a semantic summarization pass.

## Carol processing-clean-2 successful helper corroboration (read-only)

A later read-only inspection independently confirms the clean processing pass reported by the UI: run `c48fa860-966f-410e-9f32-2ea6735aad1a`, session `20260922_10`, destination channel `c874aef4-10b5-41d3-99f7-6d6c90c4e644`, and connection `8c991d2f-afef-4a3c-9d52-36647b4f76c7`. The retained run ledger records `status=completed` and `error=null`. Connection metadata records workspace `0fb6e0c6-32c0-4d0a-957e-8a76240d54c2`, owner UID `1101`, private mode, and policy epoch 6; the remote fixture helper and output are owned by Carol UID 1103. The retained session metadata records provider `ollama`, model `qwen3:8b`, privacy tier `private`, and `privacy_reason=mcp:crew`.

The four typed calls and matched responses are persisted in session `20260922_10`:

| Call | Typed request | Matched result |
| --- | --- | --- |
| `call_j9j11r59` | `remote.execute`, argv `["python3", "summarize_csv.py", "crew-task.csv", "summary.csv"]` | Job `66537cc8d2128331cf8de475a43c6ca0b2605f66d4ee13a5439a257cff4e8f67` completed later; initial receipt was `running` and `isError=false` |
| `call_6wbc60di` | `remote.job_status` for that job | `status=completed`, `exit_code=0`, stdout `row_count=3 total=60`, empty stderr |
| `call_15dpu4o6` | `remote.read` path `summary.csv` | 21 bytes, `row_count,total\n3,60\n`, SHA-256 `29f857b262032224f20e849e76bed6907e8e02846835a632c8380b2bafb91ab3` |
| `call_h3wcdehz` | `remote.attach` path/blob `summary.csv`, media type `text/csv` | `isError=false`; attachment `877d8a8e-e47c-41cd-9f9a-a03c99197f7c` and typed channel post `dcb025a6-e6e8-4401-aad2-b9e734cbe389` in the destination channel |

Read-only SSH verification against the same pinned localhost fixture found `/home/carol/work/crew-fixture/summary.csv` mode 0644, UID/GID 1103:1103, size 21, with the same bytes and digest. The live Carol daemon process uses `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter/target/debug/biorouterd`; that ELF hashes to `1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407`. No model call, profile mutation, remote write, or fixture reset was performed during this corroboration. This is a real GUI/API processing and attachment result on the restricted channel, separate from the synthetic public-provider boundary harness and earlier failed processing attempts.
