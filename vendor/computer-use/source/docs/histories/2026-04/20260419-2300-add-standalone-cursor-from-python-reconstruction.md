## [2026-04-19 23:00] | Task: 新增基于 Python 重建脚本的 StandaloneCursor

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 现在有一个 `swift run CursorMotion` 的版本，但是感觉做得很不理想。基于单独的 `scripts/cursor-motion-re/reconstruct_cursor_motion.py` 去实现一个新的 `StandaloneCursor` 版本出来看看。

### 🛠 Changes Overview
**Scope:** `Package.swift`、`experiments/StandaloneCursor/`、`README*`、`docs/`

**Key Actions:**
- **[新增独立 target]**: 在 `Package.swift` 里增加 `StandaloneCursor` executable target、`StandaloneCursorSupport` support module 和对应测试 target，避免继续改当前脏着的 `CursorMotion` 线路。
- **[Swift 版重建模型]**: 在 `experiments/StandaloneCursor/Sources/StandaloneCursorSupport/StandaloneCursorModel.swift` 里按 Python 脚本重建 `20` 条 candidate、`measure + score`、selection policy，以及 `response=1.4` / `dampingFraction=0.9` / `dt=1/240` 的 raw spring timeline。
- **[新独立 viewer]**: 新增 `StandaloneCursor` app，支持拖动起终点、切换 candidate、重放路径，并把 endpoint lock / close-enough 时间直接展示在 UI 上。
- **[验证与文档]**: 新增 `StandaloneCursorSupportTests`，并补齐 `experiments/StandaloneCursor/README.md`、顶层 README、`docs/ARCHITECTURE.md` 和执行计划。

### 🧠 Design Intent (Why)
现有 `CursorMotion` 更偏视觉和交互实验，不适合继续承载“基于 Python 脚本直接对照 binary lift”的诉求。这次把新 viewer 做成独立 target，一方面能保留旧 lab 的实验自由度，另一方面也能给后续对照脚本、继续往主运行时收敛时提供一个更干净的中间层。

### 📁 Files Modified
- `Package.swift`
- `experiments/StandaloneCursor/README.md`
- `experiments/StandaloneCursor/Sources/StandaloneCursor/StandaloneCursorApp.swift`
- `experiments/StandaloneCursor/Sources/StandaloneCursor/StandaloneCursorRootView.swift`
- `experiments/StandaloneCursor/Sources/StandaloneCursorSupport/StandaloneCursorModel.swift`
- `experiments/StandaloneCursor/Tests/StandaloneCursorSupportTests/StandaloneCursorSupportTests.swift`
- `README.md`
- `README.zh-CN.md`
- `docs/ARCHITECTURE.md`
- `docs/exec-plans/completed/20260419-standalone-cursor-from-python-reconstruction.md`
