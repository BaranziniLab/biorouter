# JS REPL Computer Use 支持

## 目标

把 Open Computer Use 从“只暴露离散的 9 个 MCP tools”扩展为可由持久 Node.js REPL 编排的 Computer Use runtime：模型通过 `js` / `js_reset` 编写异步 JavaScript，使用 app-bound API，在一次 tool round trip 内完成多步动作、条件、循环和最终状态读取。

## 范围

- 包含：
  - 调研官方 bundled `computer-use`、host `node_repl` 与 `@oai/sky` 的启动与调用链。
  - 审阅社区 PR #65、#72、#73、#74、#75，重点复用 #75 的意图并修正协议偏差。
  - 新增跨平台 Node REPL adapter，把现有 native MCP server 的离散 tools 转成异步 `cua` app-bound API。
  - 让 Codex plugin 默认通过 Node REPL adapter 暴露 `js` / `js_reset`，保留原生 `mcp` 入口兼容其他 host。
  - 覆盖持久 binding、reset、错误传播、截图输出、同一次调用内多步工具编排和 child lifecycle。
  - 同步架构、安全、可靠性、usage、plugin metadata、release note 与 history。
- 不包含：
  - 直接复制或依赖官方 proprietary `@oai/*` packages。
  - 在 Swift 中嵌入 JavaScriptCore 作为主 REPL；该方案缺少官方 Node REPL 的 async/top-level await/module/output 语义。
  - 本轮合并 #65、#72、#73、#74；它们独立评审和交付。
  - 本轮移除现有 9-tool MCP surface；`open-computer-use mcp` 保持兼容。

## 背景

- 相关文档：
  - `docs/ARCHITECTURE.md`
  - `docs/SECURITY.md`
  - `docs/RELIABILITY.md`
  - `skills/open-computer-use/references/usage.md`
- 相关代码路径：
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseToolDispatcher.swift`
  - `plugins/open-computer-use/`
  - `scripts/npm/build-packages.mjs`
- 已知约束：
  - 官方 plugin 1.0.1001068 仍是 `@oai/sky` skill + native 9-tool MCP client；本机检查到的 host bundle 提供 Node 22 `node_repl` 与 `@oai/sky` 0.6.11。
  - 已检查的官方 bundle 使用 host 提供的通用 `node_repl`，由 plugin 指引模型导入异步 `@oai/sky` API；REPL 不是 native MCP 自身新增的两个 tools。
  - Node REPL 必须有持久顶层 binding、top-level await、显式输出、超时和 reset。
  - 不得在 commit 或 PR 中加入禁止的 Co-authored-by trailer。

## 风险

- 风险：子进程 MCP 与 Node REPL 双层生命周期处理不当会泄漏 native runtime 或挂死。
  - 缓解方式：adapter 统一接管 SIGINT/SIGTERM/SIGHUP、stdin EOF、pending JSON-RPC、默认 timeout，并在测试中验证关闭路径。
- 风险：任意 JavaScript 比离散 tool surface 权限更宽。
  - 缓解方式：明确这是本地 Node execution surface；plugin 默认入口只用于 Codex 场景，原生 MCP 保持可选；文档说明文件/网络能力与信任边界。
- 风险：官方 API 仍在快速变化。
  - 缓解方式：实现稳定的最小 app-bound API，独立 adapter 与 native dispatcher，通过 contract tests 固定协议，避免耦合 proprietary package。
- 风险：截图二进制在 nested MCP/Node bridge 中膨胀。
  - 缓解方式：只在 `getScreenshot` / `getAXStateAndScreenshot` 时解码并 emit，普通动作不透传 action-result screenshot。

## 里程碑

1. 调研官方机制并审阅社区 PR。
2. 落跨平台 Node REPL adapter、plugin route 与 contract tests。
3. 文档、history、release note 和完整 CI 验证。

## 验证方式

- 命令：
  - `node --test scripts/node-repl/*.test.mjs`
  - `swift test`
  - `(cd apps/OpenComputerUseLinux && go test ./...)`
  - `(cd apps/OpenComputerUseWindows && go test ./...)`
  - `make check-docs`
  - `./scripts/run-tool-smoke-tests.sh`
  - `./scripts/ci.sh`
- 手工检查：
  - MCP `tools/list` 只有 `js` / `js_reset`。
  - 连续 `js` 调用保留顶层 binding，`js_reset` 清空。
  - `await cua.getApp(...)` 返回 app binding，动作与 snapshot 在一次 `js` 内按序执行。
- 观测检查：
  - native child 退出或超时给出结构化错误；adapter 退出时 child 不残留。

## 进度记录

- [x] 读取仓库规则与架构文档。
- [x] 逆向当前与上一代官方 plugin/runtime。
- [x] 审阅 #65、#72、#73、#74、#75 的范围、冲突和设计。
- [x] 完成 Node REPL adapter 与 plugin 集成。
- [x] 完成 contract tests 和跨平台验证。
- [x] 完成文档、history、release note 与 plan 归档。

## 决策记录

- 2026-09-21：不直接合并 #75 的 JavaScriptCore 同步 runtime。保留其“代码编排减少 round trip”的产品判断，但采用 Node.js、top-level await、persistent lexical bindings、`nodeRepl.write/emitImage` 和 app-bound async `cua` API；这是适配任意 MCP host 的开源实现，不声称官方 plugin 本身也暴露这两个 MCP tools。
- 2026-09-21：本轮采取 additive migration。Codex plugin 默认走 `js` / `js_reset` adapter；原生 `open-computer-use mcp` 继续暴露离散 tools，避免一次性破坏所有 MCP host。
- 2026-09-21：#73 的 targeted AX query 与 #74 的 App Intent 不作为 model-facing standalone tools 并入本轮；未来若采用，应优先成为 app-bound `cua` API 的能力，而非继续扩大顶层 MCP tool surface。
- 2026-09-22：完整验证通过：Node contract 12/12、Swift 167 tests（1 个 opt-in live test skipped）、Linux/Windows Go tests、Linux Python tests、9-tool 与 cursor smoke、npm dry-run/staged launcher、隔离的 Codex plugin cache install 和 macOS 真实 native bridge。Linux devbox 安装缺失的 `gir1.2-atspi-2.0` 后，另外验证了 Node 22 下的 contract、两工具 surface、结构化空 app 列表、持久 binding、CPU timeout/reset 和 native Linux bridge；该 SSH 环境没有可见的登录桌面窗口，所以 app 数量为 0。仓库级 `scripts/ci.sh` 仅在已有 repository-hygiene gate 失败：当前 checkout 的 `main` 本身缺少 `.editorconfig`、`.markdownlint.json` 及多份 `.github` 模板/workflow；本轮相关的其余 CI 子步骤已单独通过。
