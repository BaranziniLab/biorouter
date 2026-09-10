# The workspace map

> **What this is.** The per-turn file map the agent is given about its working directory: what it
> contains, what it costs, the four rules that keep its directory walk off the turn's critical
> path, and the decision that a home directory gets no map at all.
> **Status:** Current. The bounding rules landed after finding M1 of the 2026-09-10 test drive of
> `main`; the map itself is BR-1 and predates them.
> **Audience:** Contributors changing per-turn context assembly, and anyone diagnosing a chat that
> hangs before the model is reached.

## What the map is

Every turn, the agent is handed a short `<info-msg>` block — the "message of the moment", or MOIM —
carrying the current time, the working directory, and a bounded, gitignore-aware listing of that
directory. Without it the agent knows one thing about its surroundings, the working directory's
path, and rediscovers project structure by hand in every session.

The listing is produced by `agents::workspace_summary` and appended by `ExtensionManager::collect_moim`
right after the working-directory line. It is names only, depth-capped, entry-capped, token-capped and
cached, and it honours the same trust boundary as the rest of the agent's file access: `.gitignore`,
`.ignore`, and the local and global `.biorouterignore` files. Anything the user hid from the agent
stays hidden here too.

`CONTEXT_WORKSPACE_SUMMARY: false` in `config.yaml` turns the whole feature off and restores the old
one-line behaviour. The setting is read live, through the file-stamp-cached config, so it takes
effect without restarting the daemon.

## The bounds

| Setting | Default | What it bounds |
|---|---|---|
| `CONTEXT_WORKSPACE_SUMMARY` | `true` | Whether a map is injected at all. |
| `CONTEXT_WORKSPACE_SUMMARY_MAX_DEPTH` | `3` | How far below the working directory the walk descends. |
| `CONTEXT_WORKSPACE_SUMMARY_MAX_ENTRIES` | `200` | Rendered entries before the tail is elided. |
| `CONTEXT_WORKSPACE_SUMMARY_MAX_TOKENS` | `2000` | The rendered map's own token budget. |
| `CONTEXT_WORKSPACE_SUMMARY_TTL_SECS` | `30` | How stale a served map may be. |
| `CONTEXT_WORKSPACE_SUMMARY_BUDGET_MS` | `1500` | How long a turn waits for the walk before proceeding without a map. |

`SCAN_CAP` (20,000 entries) is a hard ceiling in source rather than a setting, so a directory holding
100,000 flat files cannot make the bounded walk unbounded.

## Why the walk is never on the turn's critical path

This module used to walk the tree **synchronously, on the tokio worker driving the turn**, with no
timeout, no cancellation point and no log line. On 2026-09-10 a test drive of `main` measured what
that costs. With the default working directory — `$HOME` on macOS — every turn blocked in
`std::fs::read_dir` → `__opendir2` under `~/Library/Group Containers`, which holds the Dropbox,
iCloud, GlobalProtect and Office app-group containers, and never reached the provider at all. One
hundred percent of `/usr/bin/sample` samples sat in that one stack across four captures in two
sessions. The daemon idled at 0.0% CPU while the composer said "Thinking" for eight and a half
minutes. `Stop` could not end it, because a blocking `std::fs` call has no cancellation point, and
each wedged turn leaked its worker for the life of the process — the thread and its three open
descriptors were still there twenty minutes later, after the chat had been deleted through the
interface.

A control ruled out the obvious explanation: a shell `find` over the same directory finished in 3.4
seconds and returned 11,849 entries. Only that process could not read it. Code signing was
investigated, acted on and falsified — re-signing the daemon with the Developer ID changed nothing.

Four rules now keep that from recurring. Each is load-bearing, and each is pinned by a test in
`agents::workspace_summary`.

**No filesystem syscall happens on the async path.** Not the walk, not the root's `stat`, not
`canonicalize`. Every one of them belongs to the blocking task, where a wedge costs a pool thread
instead of the turn. Two consequences follow that look like oversights and are not: the cache is
keyed on the working directory *as given* rather than on its canonical form, so two spellings of one
directory get two entries; and the fast path is TTL-only.

**The walk runs on the blocking pool with a finite budget.** When the budget lapses the turn proceeds
without a map, one WARN names the directory and the lever, and the negative result is cached for the
TTL so the next turn pays nothing at all. The budget cannot be configured to zero — a zero would read
as "wait forever", which is the bug. `CONTEXT_WORKSPACE_SUMMARY: false` is the only supported off
switch.

**The wait is tied to the turn's cancellation token.** The walk itself cannot be cancelled; the wait
for it can, so `Stop` returns immediately and leaves the walk to finish, or not, on its own. This is
why the token is threaded from `Agent::reply_internal` down through `assemble_turn_context`,
`inject_moim` and `collect_moim` — the walk is the only part of per-turn context assembly that
touches the filesystem.

**One walk per root while one is in flight.** A blocked `std::fs` call cannot be cancelled, so a
thread that wedges is lost. Single-flight is what bounds that loss to one thread per root instead of
one per turn. A walk that never returns therefore never releases its marker, and that is deliberate:
the alternative is a new doomed thread on every turn. While a walk is outstanding, later turns are
served whatever the last completed walk produced.

## A home directory is not a workspace

The rules above bound the damage. They do not make walking `$HOME` a good idea, and the decision here
is that it is not one.

A home directory is the union of every project the user owns *and* the operating system's own
per-user state. On macOS that state includes the cloud-provider containers that produced the wedge.
On any platform it is tens of thousands of entries that say nothing about the task at hand, so even
a walk that completes spends the map's entire 200-entry budget on `Desktop/`, `Downloads/` and
`Library/` before reaching anything the agent could use.

So `skip_reason` refuses three kinds of root outright, with no walk at all and no budget spent:

- **The home directory itself.** Not directories inside it: `~/code/thing` is an ordinary workspace.
- **A filesystem root** (`/`, `C:\`), for the same reason and more so.
- **Anything at or beneath an opaque tree** — `~/Library`, `~/AppData`, `~/.Trash`, and any path
  holding a `Group Containers`, `CloudStorage`, `Mobile Documents` or `FileProvider` component.
  These are places where the operating system or a sync client mediates the read, so an `opendir`
  can block on a daemon rather than on a disk.

The `Library` and `AppData` rules are matched **home-relative**, not by name alone. `Library/` is an
ordinary directory name inside a project — an R library, a component library — and stays walkable
there.

The same opaque trees are pruned *during* a walk that started somewhere legitimate, and the walk does
not cross a mount point: a network share and a File Provider volume are both mounts, and an
unresponsive one is the other way this blocks.

A skipped root logs one line naming the reason. The chat still works; it simply has no file map, and
the agent's directory tools reach the tree on demand exactly as they did before BR-1.

### Why not just default the feature off for `$HOME`?

That was the alternative considered, and it is weaker in both directions. Turning
`CONTEXT_WORKSPACE_SUMMARY` off for one root would leave the setting reading `true` while behaving as
`false`, which is the kind of divergence that costs an afternoon to diagnose; and it would say
nothing about `~/Library/CloudStorage`, which is not the home directory and wedges just as hard.
Refusing the root is narrower, states the rule where the rule belongs, and leaves the feature switch
meaning exactly one thing.

## What a turn without a map looks like

Nothing in the transcript. The MOIM block simply carries the working-directory line and no listing,
which is the pre-BR-1 behaviour. The evidence is in the daemon log, once per directory per process:

- `workspace map: no file map for this chat because …` at INFO, for a refused root.
- `workspace map: reading this directory did not finish within the budget …` at WARN, naming the
  directory and both levers, for a walk that overran.

## Tests

```bash
cargo test -p biorouter --lib -- workspace_summary
```

The behavioural tests inject the walk rather than trying to conjure a stalling directory on demand,
because a directory that reliably blocks `opendir` cannot be created from a test: a FIFO does not
stall a walk (`walkdir` only `lstat`s it), and an unresponsive mount is not something a unit test may
arrange. `summary_bounded` therefore takes the walk as a closure, and the tests supply one that
blocks until released. That makes the budget, cancellation and single-flight assertions deterministic
instead of racing a real tree.

## Related documentation

- [Turn cancellation and process reaping](turn-cancellation-and-process-reaping.md) — the rest of
  what `Stop` reaches, and the two rules that keep the cancellation chain intact.
- [Context engineering](context-engineering.md) — the other sources of durable background knowledge
  the agent is given.
- [Workspace control](workspace-control.md) — the agent's tool surface over *other* conversations,
  which is a different thing that shares the word.
