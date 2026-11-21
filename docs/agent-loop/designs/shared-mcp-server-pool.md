# Shared MCP server pool (BR-54)

> **What this is.** The design for eliminating the two axes of MCP process multiplication —
> one process tree per `Agent`, and one `biorouterd` daemon per app window — by pooling MCP
> servers behind a shared, fingerprint-keyed registry.
> **Status:** Current. Both slices shipped: Slice A (one daemon per app) as commit `4cdf3f86`
> and Slice B (`SharedMcpPool`, flag-gated) as `d856a00e`; `crates/biorouter/src/agents/mcp_pool.rs`
> is on main. This document is now the architecture reference for that live pooling code, and
> the plan of record for the one decision still open — whether Slice B flips on by default.
> **Audience:** developers working on BioRouter's extension manager, MCP client, and the
> Electron main process.

BioRouter multiplies MCP (Model Context Protocol) child processes on two independent axes that compound: every `Agent`
builds its own `ExtensionManager` and spawns its own copy of every enabled server, and every
Electron window spawns a whole additional `biorouterd`. With 100 cached agents and a handful
of 40–150 MB `uvx`/Node servers, this is the dominant RAM story. The hard part is not sharing
the process — it is keeping per-session isolation on working directory, environment, provider
and notification routing once the process *is* shared.

> **Identifier key.** `BR-NN` identifiers are proposals from the 67-item master list in
> [the agent-loop improvement proposals](../../history/agent-loop-review/improvement-proposals.md).
> `P-NN` identifiers are the numbered entries in the three lens reviews under
> [proposal lenses](../../history/agent-loop-review/proposal-lenses/); a lens is one of
> **P** (performance), **R** (robustness), or **U** (ux). This document is BR-54, raised under
> the performance lens as P-26 and P-27.

| Field | Value |
|---|---|
| Proposal | BR-54 |
| Lens | Performance (P-26, P-27) |
| Inspired by | jcode's `mcp/pool.rs` — *"one process that owns all sessions"*, `Arc<OnceCell<Arc<SharedMcpPool>>>`. See the [jcode comparison analysis](../../history/performance-2026-06/jcode-comparison-analysis.md). |
| Shipped | Slice A as `4cdf3f86`, Slice B (flag-gated) as `d856a00e`, during the [agent-loop fix campaign](../../history/agent-loop-campaign/README.md) |

> **Note — a neighbouring document carries a stale label.** The
> [mid-flight review index](../../history/agent-loop-campaign/mid-flight-review-index.md) lists
> BR-54 as "designed, not implemented". That snapshot was taken before Slice B landed; the
> index now annotates the label as stale. This document and
> `crates/biorouter/src/agents/mcp_pool.rs` are authoritative.

> **Note.** Every `file:line` citation below was taken against the pre-campaign tree, before
> the 2026-07-13 integration merge. The file paths remain accurate; the line numbers have
> since moved. Treat the paths as authoritative and the line numbers as historical anchors.

---

## The problem, grounded in code

BioRouter multiplies MCP child processes on **two independent axes**, and both are the
dominant RAM story (see the
[jcode comparison analysis](../../history/performance-2026-06/jcode-comparison-analysis.md)).

### Axis 1 — one MCP process tree per Agent (backend)

Every `Agent` builds its own `ExtensionManager`:
`crates/biorouter/src/agents/agent.rs:248` — `extension_manager: Arc::new(ExtensionManager::new(provider.clone(), session_manager))`
(inside `Agent::with_config`, which starts at `agent.rs:237`; the source proposal cites line
"236", which is this constructor). `ExtensionManager` owns `extensions: Mutex<HashMap<String, Extension>>`
(`extension_manager.rs:99`), and every enabled server is spawned into that map by
`add_extension` (`extension_manager.rs:518`). For a stdio/uvx server this reaches
`child_process_client` (`extension_manager.rs:225`) which does a real
`TokioChildProcess::builder(command)…spawn()` (`extension_manager.rs:262-264`) per agent.

The agents themselves are pooled but not deduplicated: `AgentManager` holds an
`LruCache<String, Arc<Agent>>` capped at `DEFAULT_MAX_SESSION = 100`
(`crates/biorouter/src/execution/manager.rs:14,19,84-116`), each `get_or_create_agent`
minting a fresh `Agent` (`manager.rs:100`). Worst case: **100 live agents × M stdio/uvx
servers**, each server a 40–150 MB OS process (a `uvx` Python or Node `@playwright/mcp`
child). Grep confirms **no shared pool exists** — there is no `SharedMcpPool` type today.

### Axis 2 — one whole `biorouterd` daemon per Electron window (frontend)

`createChat` (`ui/desktop/src/main.ts:862`) calls `startBiorouterd`
(`ui/desktop/src/biorouterd.ts:99`) unconditionally for **every** window;
`startBiorouterd` picks a fresh port (`biorouterd.ts:121`, `findAvailablePort`) and
`spawn`s a new daemon (`biorouterd.ts:172`). Each window then gets its own HTTP client
keyed by `mainWindow.id` and a ref-counted backend (`main.ts:958-959`,
`retainBackend`/`releaseBackend` at `main.ts:831-849`). N windows = N tokio runtimes + N
`AgentManager`s + N SQLite pools + N×(everything on Axis 1). This is pure waste: the
daemon is **already** a session-keyed singleton — `AgentManager` is a process-global
`OnceCell` (`manager.rs:16,53-67`), the server routes by `session_id` (see the
[server reply-flow review](../../history/agent-loop-review/subsystem-reviews/server-reply-flow-and-session-lifecycle.md)),
and the secret key is a stable module-level constant
(`main.ts:795-805`, `GENERATED_SECRET`). Only the Electron spawn path assumes per-window.

### Why sharing is non-trivial: the isolation surface

A shared MCP process cannot be naively reused across sessions because per-session state is
baked into the spawn and the client:

1. **Working directory.** `child_process_client` sets `command.current_dir(dir)` and
   `command.env("BIOROUTER_WORKING_DIR", dir)` (`extension_manager.rs:249-251`), where
   `dir` comes from `resolve_working_dir()` → `working_dir` set per session by
   `set_working_dir` (`extension_manager.rs:495-497`, called from
   `agent.rs:1043-1044`, `load_extensions_from_session`). Two sessions with different
   folders **must not** share a process — the cwd is fixed at spawn.
2. **Environment / secrets.** `merge_environments(envs, env_keys, …)` folds config env +
   keychain secrets into the spawned process env (`extension_manager.rs:544` etc.). Two
   sessions with different secrets must not share.
3. **Provider (MCP sampling).** `McpClient::connect(transport, timeout, provider)` captures
   a `SharedProvider` (`extension_manager.rs:275`, `mcp_client.rs:316`). The client's
   `create_message` handler runs server-initiated sampling against *that* provider
   (`mcp_client.rs:176-200`). One shared client ⇒ one provider; sessions on different
   models would sample the wrong one.
4. **Notification fan-out.** `BioRouterClient::on_progress` / `on_logging_message`
   broadcast to **every** subscriber in `notification_handlers`
   (`mcp_client.rs:135-174`). `dispatch_tool_call` subscribes per call and returns that
   stream (`extension_manager.rs:1288`, `client.subscribe()` at `mcp_client.rs:553`).
   Today that fan-out is intra-agent (all subscribers belong to one session), so it is
   harmless. Share one client across sessions and **session A receives session B's
   progress/log stream.** Note calls set `meta: None` (`mcp_client.rs:479-486`) — no
   progress token is assigned today, so there is nothing to route by yet.
5. **Elicitation** already crosses sessions: `create_elicitation` uses the global
   `ActionRequiredManager` with one shared `request_rx` and no per-session addressing
   (`mcp_client.rs:250-267`; see the
   [long-running tasks review](../../history/agent-loop-review/subsystem-reviews/long-running-tasks-and-scheduling.md)).
   Pre-existing, but the pool must not make it worse.

Net: the design must **share the OS process while keeping per-session isolation** on cwd,
env, provider, and notification routing.

---

## Design

Two loosely-coupled slices that ship independently (the proposal's own note: "groups two
closely-related RAM redesigns; can ship independently").

### Slice A — One daemon per app (frontend, no Rust change)

Make `startBiorouterd` a **process-global singleton** in the Electron main process and
have every window connect to it.

- New module state in `ui/desktop/src/main.ts` (or a small `biorouterdSingleton.ts`):
  ```ts
  let sharedBackend: Promise<BiorouterdResult> | null = null;
  function getSharedBackend(app, serverSecret, env): Promise<BiorouterdResult> {
    if (!sharedBackend) sharedBackend = startBiorouterd({ app, serverSecret, dir: os.homedir(), env, ... });
    return sharedBackend;
  }
  ```
- `createChat` (`main.ts:880`) awaits `getSharedBackend(...)` instead of calling
  `startBiorouterd` directly. All windows share one `baseUrl`; each still gets its own
  typed client (`createClient`, `main.ts:951`) with the same secret — fine, the client is
  cheap and stateless. `biorouterdClients.set(mainWindow.id, client)` stays per-window.
- **Working directory is already per-session, not per-daemon.** The daemon's spawn
  `cwd`/`BIOROUTER_WORKING_DIR` is only a *fallback* used when a session has no working
  dir; in the GUI every session sets its own via `session.working_dir` →
  `set_working_dir` (`agent.rs:1043`). So a single daemon started at `os.homedir()` serves
  windows opened on different folders correctly. The per-window `dir` continues to flow as
  `REQUEST_DIR`/`BIOROUTER_WORKING_DIR` in `appConfig` (`main.ts:923-935`) and becomes the
  session's working dir at session-create time — unchanged.
- **Lifecycle rework** (the proposal's named risk "which window kills the daemon"):
  replace the per-window `retainBackend`/`releaseBackend` ref-count (`main.ts:831-849`)
  with a single app-lifetime backend. The daemon is killed only in `app.on('will-quit')`
  (there is already a sweep there, `main.ts:4644`), and on `window-all-closed` when the
  app actually quits (`main.ts:4710`). Application Drafter launch-app windows that reuse the
  launching window's client (`main.ts:4551-4584`) become trivial — everyone shares the
  one backend. Keep the `externalBiorouterd` / `BIOROUTER_EXTERNAL_BACKEND` branches
  (`biorouterd.ts:105-112`) as-is: those already point all windows at one URL.
- Files changed: `ui/desktop/src/main.ts`, `ui/desktop/src/biorouterd.ts` (export a
  reusable result), no backend change.

### Slice B — `SharedMcpPool` inside the daemon (backend)

Introduce a process-global pool of MCP clients keyed by a **spawn-identity key** so N
agents share M processes. The pool sits *behind* each `ExtensionManager`: the manager
still owns its own `extensions` map and tool prefixing, but the `Extension.client` becomes
a **handle into the pool** rather than a privately-owned process.

**New module:** `crates/biorouter/src/agents/mcp_pool.rs`.

```rust
// Identity that must match for two sessions to safely share ONE process.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    transport: TransportKey,          // Stdio{cmd,args} | Http{uri} | Builtin{name} | InlinePython{code_hash}
    working_dir: Option<PathBuf>,     // from resolve_working_dir(); None => process cwd
    env_fingerprint: u64,             // stable hash of merged (envs + resolved env_keys) incl. secrets
}

pub struct SharedMcpPool {
    entries: Mutex<HashMap<PoolKey, Weak<PooledClient>>>,   // Weak => auto-evict on last release
}

pub struct PooledClient {
    key: PoolKey,
    inner: McpClientBox,                                    // the real Arc<Mutex<Box<dyn McpClientTrait>>>
    router: Arc<NotificationRouter>,                        // per-(session,progress-token) fan-out
    _temp_dir: Option<TempDir>,                             // held for InlinePython lifetime
}

impl SharedMcpPool {
    pub fn global() -> &'static Arc<SharedMcpPool>;         // OnceCell, mirrors AgentManager::instance

    /// Return an existing live client for `key`, else build one via `spawn` and insert.
    /// Concurrency-safe: single-flight per key so two agents connecting the same server
    /// at once share ONE spawn (not two racing processes).
    pub async fn get_or_spawn<F, Fut>(&self, key: PoolKey, spawn: F) -> ExtensionResult<Arc<PooledClient>>
    where F: FnOnce() -> Fut, Fut: Future<Output = ExtensionResult<McpClientBox>>;
}
```

**Pool key derivation.** `ExtensionConfig::key()` is *name-only* today
(`extension.rs:458-463`), which is wrong for sharing (two servers named "files" with
different cmds/dirs would collide). Add `ExtensionConfig::pool_key(working_dir, env) ->
PoolKey` in `extension.rs` that hashes the transport-defining fields + resolved
working_dir + `env_fingerprint`. Built-in and Platform servers key on `name` (they are
cheap in-process duplex servers, `extension_manager.rs:583-627`); stdio/uvx/http key on
their full spawn tuple.

**Notification isolation (the correctness core).** Assign a **per-dispatch progress token**
and route by it:

- `dispatch_tool_call` (`extension_manager.rs:1288`) generates a unique token, sets it in
  `CallToolRequestParams.meta.progressToken` (currently `meta: None`,
  `mcp_client.rs:479-486` — wire it through `call_tool`), and registers a bounded
  `mpsc::Sender` in `PooledClient.router` under that token.
- `BioRouterClient::on_progress` (`mcp_client.rs:135-153`) is changed from
  "broadcast to all handlers" to "look up the token in `context`/`params.progress_token`
  and send to that one subscriber." MCP requires the server to echo the request's progress
  token on progress notifications, so this is spec-clean.
- `on_logging_message` (`mcp_client.rs:155-174`) has **no** token binding. Options
  (see open question 1): (a) drop shared-client logging notifications to the caller and rely on
  server stderr logs, or (b) keep a per-session broadcast only among sessions sharing the
  key and accept low-stakes log text bleed. Recommend (a) for first slice — logging
  notifications are diagnostic, not user-facing progress.
- Because `McpMeta.session_id` is already injected per call
  (`mcp_client.rs:471-489`, `dispatch_tool_call` builds `McpMeta::new(&session_id)`), the
  *tool call itself* is already session-scoped — only the *notification* path needs the
  new token routing. This is the key reason sharing is feasible without touching every MCP
  server.

**Provider for sampling.** The shared `PooledClient` holds one `SharedProvider`. Rather
than pin it to the first session, thread the caller's provider via the same per-call
`McpMeta` seam so `create_message` (`mcp_client.rs:176`) resolves the *current* session's
provider from a task-local (there is already `crate::session_context::current_session_id()`
used at `mcp_client.rs:580-582`; add a parallel `current_provider()` set for the duration
of a dispatched call). Servers that never sample (the common case) are unaffected.

**Wiring into ExtensionManager (minimal surface).** `add_extension`
(`extension_manager.rs:518`) keeps its per-variant match but, instead of calling
`child_process_client`/`McpClient::connect` inline, calls
`SharedMcpPool::global().get_or_spawn(key, || <the existing spawn closure>)`. The returned
`Arc<PooledClient>` is stored in `Extension.client` (as an `McpClientBox` view). On
`remove_extension` (`extension_manager.rs:774`) and on agent drop, the manager drops its
`Arc<PooledClient>`; when the last agent releases it, the `Weak` in the pool goes dead and
the process is reaped. Per-app in-process servers (`add_inprocess_server`,
`extension_manager.rs:700`) stay **unpooled** — they carry per-app context and are
explicitly excluded from `get_extension_configs` (`extension_manager.rs:796-799`); the
pool only covers registry-spawnable configs.

**Control flow (call path, shared):**

```text
Agent::reply → dispatch_tool_call(session_id, call, cancel)
  → get_client_for_tool(prefix)                 [unchanged: extension_manager.rs:973]
  → PooledClient.inner (shared Arc)
  → assign progress_token; router.register(token → tx)
  → session_context::scope(session_id, provider) {
        client.call_tool(name, args, McpMeta::new(session_id), cancel)   [mcp_client.rs:471]
    }
  → notifications routed by token → only this session's stream
```

---

## Alternatives considered, and why they were rejected

- **Per-session process, just cap the LRU harder.** Lowering `DEFAULT_MAX_SESSION` (100)
  trims worst case but doesn't fix the M multiplier and evicts live sessions the user
  cares about. Rejected: treats the symptom.
- **Migrate working-dir from spawn-time cwd to a per-call parameter** (so *all* sessions
  can share one process regardless of folder). Widest sharing, but requires every MCP
  server — including third-party `uvx`/Node servers we don't control — to honor a per-call
  working dir. They won't. Rejected for stdio; working_dir stays in `PoolKey`. (Built-in
  in-process servers already receive the dir at spawn and could later move to per-call.)
- **Multiplex one client with logical channels per session (full session mux).** Cleanest
  isolation but a large rewrite of `McpClient` + notification plumbing. Rejected for the
  first slice; the progress-token router achieves the needed isolation far more cheaply.
- **Broadcast notifications to all key-sharers and filter in the agent.** Simpler than
  token routing but leaks progress text and needs a filter anyway. Rejected — token
  routing is barely more code and is correct.
- **One daemon per app via a lock-file/port-file discovery handshake between Electron
  processes.** Only needed if multiple OS processes host windows; Electron runs all
  windows in one main process, so a module-level singleton (Slice A) suffices. Rejected as
  over-engineering. (The `second-instance` path, `main.ts:390`, already funnels new
  invocations into the one main process.)

---

## Migration and compatibility

- **Config:** no schema change. `ExtensionConfig` gains a derived `pool_key()`; the
  persisted `EnabledExtensionsState` and `~/.config/biorouter/config.yaml` are untouched.
- **Persisted state:** none. The pool is purely in-memory. Sessions still persist their
  extension configs (`get_extension_configs`, unchanged) and replay them on resume; replay
  now hits the pool and reuses a live process instead of spawning.
- **Rollout / kill-switch:** gate Slice B behind an env flag
  `BIOROUTER_SHARED_MCP_POOL` (default off for the first release, flip to on after
  soak). When off, `get_or_spawn` degrades to "always spawn, never share" — byte-identical
  to today's behavior — so the risky path is opt-in. Slice A ships behind no flag but is
  trivially revertible (the singleton wrapper).
- **CLI vs daemon:** the CLI builds one agent per process, so pooling is a no-op win there
  (M processes for 1 agent = same as today); the pool still centralizes reaping. No CLI
  behavior change.
- **Reaping/orphans:** pool eviction must `kill` the child on last release. Reuse the
  existing detached-process discipline; ensure `Weak`-triggered drop actually terminates
  the `TokioChildProcess` (it holds the child handle) so we don't leak on eviction.

---

## Test plan

**Slice A (frontend):**
- Unit: `getSharedBackend` returns the same `BiorouterdResult` across N `createChat` calls
  (mock `startBiorouterd`, assert called once). New test file under
  `ui/desktop/src/__tests__/`.
- Manual/E2E: open 3 windows on different folders; assert exactly one `biorouterd` PID and
  that each window's chat runs in its own folder (shell tool `pwd` per session). Close
  windows in any order; daemon dies only on app quit. Covers the "which window kills the
  daemon" risk.

**Slice B (backend):**
- Unit (`mcp_pool.rs`): `get_or_spawn` single-flights (two concurrent calls same key ⇒ one
  spawn); different `working_dir` ⇒ different processes; identical config+dir+env ⇒ shared
  `Arc`; last-release drops the `Weak` and reaps the child.
- Isolation regression (`extension_manager.rs` tests, `#[cfg(test)]` block at
  `extension_manager.rs:1539`): two agents share one built-in server; concurrent
  `dispatch_tool_call`s with progress notifications — assert session A's notification
  stream receives **only** A's tokens, never B's. This is the test that proves the
  correctness core; it must fail on a naive broadcast.
- Provider isolation: shared client, session A on provider X, session B on provider Y;
  trigger `create_message` (sampling) from each; assert each used its own provider.
- Existing suites must stay green with the flag on:
  `cargo test -p biorouter agents::extension_manager`,
  `cargo test -p biorouter-mcp --test mcp_integration_test`, and the MCP cassette tests
  (`BIOROUTER_RECORD_MCP` VCR). No re-record expected (wire protocol adds a
  `progressToken` field only).
- Memory smoke: spawn 10 sessions with the same stdio extension; assert 1 child process
  (flag on) vs 10 (flag off) via `/proc` or `pgrep` count.

---

## Effort and phasing

- **Phase 1 (first mergeable slice) — Slice A, one daemon per app. Shipped as `4cdf3f86`.**
  Frontend-only,
  ~1 file, no isolation hazards (server is already singleton), biggest per-user multiplier
  (windows). Ships behind a trivial revert. This is the recommended first PR.
- **Phase 2 — `SharedMcpPool` for in-process built-ins + HTTP servers. Shipped as `d856a00e`
  (flag-gated).** These are
  stateless w.r.t. cwd and low isolation risk; validates the pool + progress-token router
  end to end behind `BIOROUTER_SHARED_MCP_POOL`.
- **Phase 3 — extend the pool to stdio/uvx/InlinePython** (the 40–150 MB processes, the
  real RAM prize), now that keying + notification routing are proven. Includes the
  provider-per-call sampling seam.
- **Phase 4 — flip the flag on by default** after soak; delete the per-window
  `retainBackend` ref-count. **Still pending** — see open question 4.

Overall effort **L**, matching the proposal, but the first slice is **S** and independently
valuable.

---

## Open questions, and how the campaign answered them

> **Note.** These are genuine product decisions, recorded as open when the design was
> written. On 2026-07-13 the campaign owner signed off with a blanket "proceed with all of
> the default options" (logged in the
> [campaign README](../../history/agent-loop-campaign/README.md)), which settled question 4
> for the shipping release: Slice B **shipped flag-gated, not on by default**. The default-on
> flip is still an open decision for a future release, and questions 1–3 remain open.

1. **Logging-notification bleed on shared clients.** Drop MCP `logging` notifications to
   the caller entirely (rely on server stderr), or keep them and accept that sessions
   sharing a process may see each other's diagnostic log lines? (Progress is routed
   precisely; only untokened logging is ambiguous.)
2. **Sharing across users/security contexts.** Should the pool key ever be allowed to
   match across sessions with *different secrets/vault contexts*, or must `env_fingerprint`
   always force isolation? (Recommend always isolate; confirm no institutional deployment
   needs looser sharing.)
3. **Idle eviction policy.** Keep a shared process alive as long as any agent references it
   (current design), or also idle-unload a process after T seconds even while referenced,
   to bound RAM on long-lived-but-idle sessions (jcode idle-unloads its embedder)?
4. **Default-on timing.** Is a flagged opt-in for one release acceptable, or should Slice B
   ship on by default given the RAM urgency? *Answered for this release: flagged opt-in.*

---

## Related documentation

- [jcode comparison analysis](../../history/performance-2026-06/jcode-comparison-analysis.md) — the source measurement and the `mcp/pool.rs` design this borrows from.
- [Campaign outcome report](../../history/agent-loop-campaign/outcome-report.md) — records `SharedMcpPool` as built and flag-gated.
- [Long-running tasks and scheduling review](../../history/agent-loop-review/subsystem-reviews/long-running-tasks-and-scheduling.md) — the pre-existing cross-session elicitation leak this design must not worsen.
- [Server reply-flow and session-lifecycle review](../../history/agent-loop-review/subsystem-reviews/server-reply-flow-and-session-lifecycle.md) — why the daemon is already session-keyed, which is what makes Slice A safe.
- [Extension manager reference](../../extensions/built-in/extension-manager.md) — the subsystem this pool sits behind.
