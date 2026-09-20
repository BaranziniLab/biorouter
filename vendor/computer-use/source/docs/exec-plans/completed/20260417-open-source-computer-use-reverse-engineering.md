# Open Source Computer Use Reverse Engineering

## 目标

为 `open-codex-computer-use` 沉淀一套可追溯的逆向分析资料，先把 `SkyComputerUseClient`、`Codex Computer Use.app`、两者之间的宿主依赖与运行时边界分析清楚，再据此设计一版可开源实现。

## 范围

- 包含：
  - 分析闭源 `Codex Computer Use.app` bundle 结构、标识、权限与运行时行为。
  - 分析 `SkyComputerUseClient` 的入口、MCP 暴露方式、宿主依赖与失败模式。
  - 把结论持续沉淀到仓库 `docs/` 下的独立目录。
- 不包含：
  - 当前阶段不开始实现开源版代码。
  - 当前阶段不承诺完全复刻官方私有协议或 UI。

## 背景

- 相关文档：
  - `docs/references/codex-computer-use-reverse-engineering/README.md`
  - `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
  - `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
  - `docs/references/codex-computer-use-reverse-engineering/packaging-and-lifecycle-integration.md`
- 相关代码路径：
  - `~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/`
  - `~/.codex/config.toml`
- 已知约束：
  - 官方 bundle 为闭源二进制，只能通过 bundle 结构、符号、strings、配置与本机运行痕迹做逆向分析。
  - 当前 `SkyComputerUseClient mcp` 不能被任意外部 MCP client 稳定直连。

## 风险

- 风险：把 strings/符号里的能力误判成当前真的启用的功能。
- 缓解方式：所有文档都区分“已观察事实”和“推断”，并尽量附上本机证据来源。

- 风险：把 Inspector 连接失败简单归因于单一原因。
- 缓解方式：同时记录 direct launch、Inspector、analytics、crash report 四类证据，避免过度下结论。

- 风险：后续实现过早锁死到官方私有架构。
- 缓解方式：持续分离“官方实现细节”和“开源版真正需要的能力边界”。

## 里程碑

1. 调研与方案收敛。
2. 逆向分析 client / service / IPC / 权限模型。
3. 基于分析结果收敛开源版架构与实现范围。

## 验证方式

- 命令：
  - `plutil -p .../Info.plist`
  - `otool -L ...`
  - `strings ... | rg ...`
  - `sqlite3 ~/Library/Group\ Containers/.../Analytics.db ...`
  - `codesign -dvv ...`
- 手工检查：
  - 对照 Inspector 连接现象与本地 crash report。
  - 对照 `~/.codex/config.toml` 与 `.mcp.json` 的 transport 配置。
- 观测检查：
  - 关注 `Analytics.db` 中的 service/client launch 事件。
  - 关注 `~/Library/Logs/DiagnosticReports/` 中的 crash 记录。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-17：先把长期逆向分析结果沉淀到 `docs/references/codex-computer-use-reverse-engineering/`，而不是分散在聊天上下文里。这样后续实现开源版时可以直接引用仓库内文档。
- 2026-04-17：当前先不实现代码，先把 client / service / host dependency 的边界分析清楚，避免一开始就沿着错误假设实现。
- 2026-04-17：曾在仓库里临时初始化 `uv` 和 Python `mcp` 依赖，用最小 Python SDK 复现实验验证 `stdio` 连接行为，而不是继续依赖 Inspector。
- 2026-04-17：上述 Python / `uv` 复现实验完成后，不再把这套一次性探针和运行链路保留在仓库里；长期保留的是文档中的证据和结论。
- 2026-04-17：把插件打包结构、`turn-ended` notify 生命周期和主 app 分发形态单独拆成文档，不和 runtime 宿主依赖混写，方便后续直接映射成开源版的接口边界。
- 2026-04-17：逆向分析阶段在仓库内收敛完成，后续实现转入 `docs/exec-plans/completed/20260417-open-source-swift-computer-use.md` 对应的 Swift 开源实现计划继续推进。
