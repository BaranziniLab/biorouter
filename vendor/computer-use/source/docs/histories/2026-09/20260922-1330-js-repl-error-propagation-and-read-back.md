## [2026-09-22 13:30] | Task: Fix JS REPL error propagation and skip action read-back

### 🤖 Execution Context
* **Agent ID**: `Claude Code`
* **Base Model**: `Claude Opus 5`
* **Runtime**: `local macOS repository workspace`

### 📥 User Query
> Benchmarked the merged `js` adapter (#78) against an in-process runtime on Calculator: a ten-click batch took ~6.5 s, and any exception in `js` code hung until the 30 s timeout and reset the session. Fix both, minimally, within the adapter-over-native-MCP design.

### 🛠 Changes Overview
**Scope:** Node.js REPL adapter, macOS `ComputerUseService`, tests, and documentation.

**Key Actions:**
- **Error propagation**: A throw inside evaluated code (sync, or after an `await`) never reached the REPL eval callback; Node prints it and moves on, so the call waited for `timeout_ms` and then reset the session. The session now settles the active evaluation from the REPL's error path (`handleError` on Node 26, the domain `error` event on Node 22) and returns `isError` with the message; bindings survive.
- **Error ownership (review follow-up)**: settling "the active call" let a late throw from a finished call (`setTimeout(() => { throw ... }, 50)`) reject whichever call ran next, and the early-rejected call kept running and wrote into the following call's output. Each evaluation now runs inside an `AsyncLocalStorage` owner that timers and promises carry; the error path settles only the call that threw, and `nodeRepl` output is refused to any code whose call has returned. Late errors from finished calls are dropped, as the REPL did before.
- **Cross-realm error messages**: errors thrown inside the REPL context fail `instanceof Error`; `asError` now uses their `message` instead of `String(error)`, avoiding `Error: Error: ...`.
- **Read-back switch**: `OPEN_COMPUTER_USE_ACTION_READ_BACK=0` makes macOS action tools return a short status and skip the post-action settle and snapshot. All action-result sites route through one `actionResult(for:)` helper; the settles that exist only for the read-back go through `pauseBeforeReadBack(_:)`. The flag is read per call so the app agent's per-request environment forwarding applies. Default behaviour is unchanged.
- **Adapter**: launches the native runtime with the variable set, since `app.click()` and friends discard the native result and state is read explicitly with `getAXState()`.
- **Tests**: Node test for thrown errors settling the call and keeping bindings (fails with a 1 s timeout before the fix); a late-throw regression test for both the in-process and Worker sessions; XCTest for the flag parsing.

### 🧠 Design Intent (Why)
The adapter's value is batching actions and one final observation in a single round trip. Each native action was still paying a 150 ms settle plus a full accessibility snapshot that the adapter threw away, so a batch cost more than the discrete tools it replaced. Skipping the read-back only when a program asks for it keeps the nine-tool surface identical for direct MCP clients. The error fix keeps a model's mistake at one failed call instead of a timeout plus a lost session.

Measured on Calculator.app (one `js` call, 5 reps, median ms): clear×2 + click "7" + read 2295 → 402; clear×2 + click "7" ×10 + read 6469 → 405; thrown exception 30000 → 0 with bindings intact.

Linux and Windows runtimes still read back after each action; the same switch would go in their `actionResult` paths and runtime scripts.

### 📁 Files Modified
- `scripts/node-repl/open-computer-use-repl.mjs`
- `scripts/node-repl/open-computer-use-repl.test.mjs`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/references/js-repl.md`
- `docs/releases/feature-release-notes.md`
