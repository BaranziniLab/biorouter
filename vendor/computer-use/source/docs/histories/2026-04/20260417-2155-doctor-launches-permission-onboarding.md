## [2026-04-17 21:55] | Task: 让 doctor 缺权限时拉起授权页

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> `open-computer-use doctor` 现在打印 `Permissions: accessibility=missing, screenRecording=missing` 时，需要跳出授权页。

### 🛠 Changes Overview
**Scope:** `apps/OpenComputerUse`、`packages/OpenComputerUseKit`、`docs/`、`scripts/npm`

**Key Actions:**
- **补 doctor 后续动作**：`doctor` 打印权限状态后，如果仍有缺失，会直接拉起现有的权限 onboarding 窗口。
- **补可测试诊断结果**：在 `PermissionDiagnostics` 增加 `missingPermissions`，让 CLI 决策复用权限层结果，并补对应单测。
- **同步用户文档**：更新仓库 README、架构文档、稳定性文档和 npm README 模板，明确 `doctor` 缺权限时会进入 onboarding。

### 🧠 Design Intent (Why)
仓库已经有完整的权限 onboarding UI，但 `doctor` 之前只负责打印结果，用户从 CLI 看到 `missing` 后还要自己再找入口。直接在缺权限时复用现有 onboarding，可以把诊断和修复收成一条更短的路径，同时避免在 CLI 里再复制一套单独的权限引导逻辑。

### 📁 Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Permissions.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/RELIABILITY.md`
- `scripts/npm/build-packages.mjs`
