## [2026-04-17 13:25] | Task: 实现开源 Swift computer-use

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI / Swift 6.2.4 / macOS`

### 📥 User Query
> 基于已经完成的 `codex-computer-use` 分析资料，实现一个 Swift 开源版本；把 todo 落到 `docs/`，持续推进实现，并把 9 个 tools 都测试好。

### 🛠 Changes Overview
**Scope:** `apps/OpenCodexComputerUse`, `apps/OpenCodexComputerUseFixture`, `apps/OpenCodexComputerUseSmokeSuite`, `packages/OpenCodexComputerUseKit`, `docs/`, `scripts/`

**Key Actions:**
- **[实现 Swift MCP server]**: 新增 Swift Package、`stdio` JSON-RPC transport、9 个 tool schema 和 `ComputerUseService`。
- **[实现本地 macOS automation]**: 接入 app discovery、snapshot、输入模拟、`doctor` / `snapshot` 诊断入口。
- **[补齐 deterministic smoke path]**: 新增 fixture app、fixture bridge 和 smoke suite，覆盖 9 个 tools 的端到端验证。
- **[收敛真实 app snapshot]**: 修正普通 app 的 AX frame 坐标换算，让 Finder 这类真实 app 输出稳定的 window-relative frame，并补充真实 app 手工验证。
- **[同步文档]**: 更新 README、架构、质量评分、安全/稳定性说明、release note 和 execution plan。

### 🧠 Design Intent (Why)
这次优先交付的是“开源可运行实现 + 可重复验证闭环”，而不是继续停留在闭源逆向分析或过早复刻官方私有宿主边界。对真实 app 保留 AX / screenshot / CGEvent 路径，对 fixture 增加测试专用 bridge，是为了同时兼顾能力真实性和回归稳定性。

### 📁 Files Modified
- `Package.swift`
- `apps/OpenCodexComputerUse/Sources/OpenCodexComputerUse/main.swift`
- `apps/OpenCodexComputerUseFixture/Sources/OpenCodexComputerUseFixture/main.swift`
- `apps/OpenCodexComputerUseSmokeSuite/Sources/OpenCodexComputerUseSmokeSuite/main.swift`
- `packages/OpenCodexComputerUseKit/Sources/OpenCodexComputerUseKit/`
- `packages/OpenCodexComputerUseKit/Tests/OpenCodexComputerUseKitTests/OpenCodexComputerUseKitTests.swift`
- `scripts/run-tool-smoke-tests.sh`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/QUALITY_SCORE.md`
- `docs/SECURITY.md`
- `docs/RELIABILITY.md`
- `docs/exec-plans/active/20260417-open-source-swift-computer-use.md`
- `docs/releases/feature-release-notes.md`

### ➕ Follow-up Progress
- **[增加 app 模式权限引导]**: `OpenCodexComputerUse` 现在不带子命令启动时会进入权限 onboarding 窗口，支持 `Accessibility` / `Screen & System Audio Recording` 深链、drag tile 和 `.app` 打包。
- **[收敛权限状态判断]**: 权限卡片不再只依赖进程内 runtime API，而是加入 TCC 持久授权记录读取，避免 GUI app 与 CLI 子进程看到不同状态。
- **[补功能验证]**: 本轮再次跑通 `swift test`、`./scripts/run-tool-smoke-tests.sh`、`doctor` 和真实 `System Settings snapshot`，确认 9 个 tools 的功能路径仍然正常。
- **[收紧权限窗口视觉密度]**: 缩小 onboarding 窗口、标题和卡片字号，收紧卡片高度与间距，并把缺失权限态的 `Allow` 按钮改成更贴近参考图的自绘胶囊按钮；同时把卡片副文案压缩到更短的表达，减少整体膨胀感。

### 🔎 Additional Files
- `apps/OpenCodexComputerUse/Sources/OpenCodexComputerUse/OpenCodexComputerUseMain.swift`
- `apps/OpenCodexComputerUse/Sources/OpenCodexComputerUse/PermissionOnboardingApp.swift`
- `packages/OpenCodexComputerUseKit/Sources/OpenCodexComputerUseKit/Permissions.swift`
- `scripts/build-open-codex-app.sh`
- `docs/exec-plans/active/20260417-permission-onboarding-app.md`
