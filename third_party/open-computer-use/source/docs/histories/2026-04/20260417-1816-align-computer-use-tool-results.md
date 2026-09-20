## [2026-04-17 18:16] | Task: 对齐 computer-use 的 tool schema 与结果载荷

### 🤖 Execution Context
* **Agent ID**: `primary`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI + SwiftPM`

### 📥 User Query
> 基于前面抓到的官方 `computer-use` / `open-computer-use` dump，对照两边的 MCP 调用参数和返回做优化。明确要求包括：
> 1. action 后的截图不要写磁盘，而是直接通过 BASE64 返回。
> 2. tools 的描述和参数要和官方严格对齐。
> 其他差异也根据实际 dump 继续优化。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit`, `apps/OpenComputerUse`, `apps/OpenComputerUseSmokeSuite`, `docs`

**Key Actions:**
- **[Schema 对齐]**: 把 9 个 tools 的 description 和 input schema 文案收敛到当前官方 `computer-use` 暴露给模型的 surface。
- **[结果载荷对齐]**: 新增 MCP tool result content 封装，让 `get_app_state` 和动作类 tools 返回 `text + image/png(base64)`，不再把普通 app 截图路径塞进文本里。
- **[状态文本收口]**: 去掉开源实现自己的 `Screenshot:` 路径和 `<element_index>` 附加块，让 state 文本更接近官方 `computer-use` 的 `App=/Window=/tree` 结构。
- **[错误语义调整]**: 将 `appNotFound("...")` 这类官方常见恢复型结果按普通 tool text 返回，而不是一律作为 MCP error。
- **[Smoke 稳定性]**: 修复 fixture state 文件的非原子写入竞争，并让 smoke suite 启动前主动清理旧 fixture 进程与旧状态文件，避免残留进程互相污染。

### 🧠 Design Intent (Why)
这次优化的目标不是“做一个功能相近的开源版”，而是尽量把 host 实际看到的 tool shape、tool result shape 和恢复语义都收敛到官方 `computer-use` 当前的调用习惯上。这样后续无论是抓包对比、做 eval，还是继续优化焦点策略，都能建立在更接近真实 host 行为的兼容面上，而不是被自定义 schema、磁盘截图路径或测试夹具竞态这些非本质差异干扰。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ToolDefinitions.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ToolResult.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Errors.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/FixtureBridge.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `docs/ARCHITECTURE.md`
- `docs/SECURITY.md`

### 🔁 Follow-up | 2026-04-17 19:01

**Additional Actions:**
- **[Official source discovery]**: 继续通过 `computer-use-cli` 和本机二进制字符串检查，确认官方 `list_apps` 的 `uses`/排序来自 Spotlight metadata 的 `kMDItemUseCount` 与 `kMDItemLastUsedDate_Ranking`，而不是 `Knowledge/knowledgeC.db`。
- **[App list exact match]**: 重写 `AppDiscovery` 的 app catalog 逻辑，按标准 application scope 做 metadata query，再和运行中 app 合并；本机实测 `list_apps` 与官方当前输出做到无 diff。
- **[Safety parity]**: 新增一层官方风格的高风险 bundle denylist，让 `iTerm2` 这类 name query 返回 `appNotFound(...)`、bundle-id query 返回 safety denial，和官方边界行为对齐。
- **[State rendering tightening]**: 继续收口 `get_app_state` 的 AX 渲染顺序、traits/value 文案、toolbar/group 展示与 secondary action 过滤；复杂 outline 场景已明显更接近官方，但 Activity Monitor 这类样本仍有少量尾部细节差异。
- **[Docs sync]**: 同步更新架构、安全、质量和 execution plan 文档，避免仓库继续保留已失效的 `Knowledge` 推测和“尚无安全策略”的旧说法。

**Extra Files Modified:**
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AppDiscovery.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `docs/QUALITY_SCORE.md`
- `docs/exec-plans/active/20260417-official-tool-alignment.md`

### 🔁 Follow-up | 2026-04-17 20:30

**Additional Actions:**
- **[State text prefix tightening]**: 把 `get_app_state` / action tool 的文本头部进一步收口到直接从 `App=<bundle-id> (pid ...)` 开始，不再输出 `Computer Use state (CUA App Version: 750)` 或 `<app_state>` 包裹。
- **[Selected text formatting parity]**: 把选中文本从我们自己的 code fence + explanatory note 改成官方当前更接近的单行 `Selected text: [...]` 形式。
- **[Canonical app identifier in errors]**: 对依赖 window bounds 的错误提示优先输出 bundle identifier，减少后续 tool 调用又漂回应用名。
- **[Regression coverage]**: 新增单测锁定 state 文本起始格式和 `Selected text` 渲染，避免后续重构把这些差异带回来。
- **[Reference correction]**: 更新仓库内 reverse-engineering 样本文档，明确当前应以直接 MCP `content[0].text` 的 `App=...` 起始格式作为官方基线。

**Extra Files Modified:**
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/references/codex-computer-use-reverse-engineering/tool-call-samples-2026-04-17.md`
