## [2026-09-01 10:20] | Task: 隔离嵌入式 App Agent Socket

### 🤖 Execution Context
* **Agent ID**: `Codex desktop`
* **Base Model**: `GPT-5`
* **Runtime**: `macOS arm64`

### 📥 User Query
> 修复 Electron 内置 OCU 在运行期间被其他 OCU App Agent Socket 争用而导致页面动作连接关闭的问题；不得影响用户的全局 OCU。

### 🛠 Changes Overview
**Scope:** macOS App Agent proxy、Socket path contract、单元测试和架构/安全文档。

**Key Actions:**
- **[Optional namespace]**: 增加 `OPEN_COMPUTER_USE_AGENT_SOCKET_NAMESPACE`。未设置或为空时严格沿用 `open-computer-use-agent.sock`。
- **[Private socket]**: 设置 namespace 时，以 SHA-256 摘要前 16 个十六进制字符派生短 Socket 文件名，不泄露原值。
- **[Verification]**: 单元测试覆盖默认兼容、确定性和 namespace 间隔离；构建 `OpenComputerUse` 成功。

### 🧠 Design Intent (Why)
不同 OCU bundle 过去共享一个固定 Socket；当一个 bundle 发现 Socket 中的 Agent 来自另一个 bundle 时会终止它。可选 namespace 让嵌入式宿主独占自己的 Agent，而不改变未配置 namespace 的现有 CLI、MCP 或全局安装行为。

### ✅ Verification
- `swift test --filter OpenComputerUseKitTests/testAppAgentSocketFileName`：2 tests passed。
- `swift build --product OpenComputerUse`：passed；仅有既存的 `nonisolated(unsafe)` warning。

### 📁 Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/MacOSAppAgentProxy.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AppAgentSocketNamespace.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/SECURITY.md`
