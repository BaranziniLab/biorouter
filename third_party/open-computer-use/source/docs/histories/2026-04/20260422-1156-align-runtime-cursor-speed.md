## [2026-04-22 11:56] | Task: 对齐 runtime overlay cursor 移动速度

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5.4`
* **Runtime**: `Codex CLI on macOS + SwiftPM`

### User Query
> `open-computer-use` 里的 overlay cursor 移动速度好像太快；对照官方速度和 `Cursor Motion` 里的可调速度。

### Changes Overview
**Scope:** `OpenComputerUseKit` cursor motion timing、文档、测试

**Key Actions:**
- **[确认差异]**: `Cursor Motion` 默认档已经对齐官方 `response=1.4 / damping=0.9` 的 `343 / 240 = 1.4291667s` endpoint-lock 时间，但 runtime 仍使用旧的距离压缩公式，实际常落在 `0.23s+`，导致中长距离移动明显偏快。
- **[Runtime timing 对齐]**: `OfficialCursorMotionModel.calibratedTravelDuration` 改为直接返回 recovered close-enough 时间，不再按路径距离和曲率压缩 wall-clock duration。
- **[回归测试]**: 新增测试锁定 runtime travel duration 等于 recovered endpoint-lock timing，避免后续重新引入距离压缩。
- **[文档同步]**: 架构文档和逆向 motion model 文档更新为默认 move 时长对齐 `343 / 240`。

### Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/CursorMotionModel.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
- `docs/histories/2026-04/20260422-1156-align-runtime-cursor-speed.md`
