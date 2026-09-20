# 隔离嵌入式宿主的 macOS App Agent Socket

## 目标

为 Open Computer Use 的 macOS App Agent 增加一个可选的、确定性的 Socket namespace。未配置 namespace 的所有既有调用继续使用历史 Socket 路径；配置 namespace 的嵌入式宿主使用私有 Socket，避免不同 OCU bundle 互相终止 App Agent。

## 范围

- 包含：Socket 文件名解析、默认兼容行为、单元测试、版本发布前的构建验证和历史记录。
- 不包含：修改或停止用户全局/NVM OCU、改变 App Agent 权限模型、重试非幂等页面动作。

## 背景

- 相关文档：`docs/ARCHITECTURE.md`、`docs/REPO_COLLAB_GUIDE.md`、`docs/HISTORY_GUIDE.md`。
- 相关代码路径：`apps/OpenComputerUse/Sources/OpenComputerUse/MacOSAppAgentProxy.swift`、`packages/OpenComputerUseKit`。
- 已知约束：历史版本固定使用 `open-computer-use-agent.sock`；当两个不同 bundle 使用该 Socket 时，后启动者会终止先启动的 Agent。Boss 筛简历必须使用内置 OCU，不能依赖或干扰全局 OCU。

## 风险

- 风险：更改默认路径会破坏已有 CLI/MCP 调用。
- 缓解方式：namespace 未设置或为空时严格返回历史文件名；仅对显式 namespace 使用新的短哈希文件名。
- 风险：Socket 路径过长或泄露宿主数据目录。
- 缓解方式：只使用 SHA-256 摘要的前 16 位，不把原 namespace 写入文件名或诊断。

## 里程碑

1. 确认 Socket 冲突路径和向后兼容边界。
2. 实现可选 namespace 并覆盖默认、确定性和隔离性测试。
3. 构建、记录历史，并等待获准发布新的运行时版本后供嵌入式宿主升级。

## 验证方式

- 命令：`swift test --filter OpenComputerUseKitTests/testAppAgentSocketFileName`。
- 命令：`swift build --product OpenComputerUse`。
- 手工检查：未设置环境变量时文件名仍是 `open-computer-use-agent.sock`。
- 观测检查：两个不同 namespace 对应不同 Socket 文件名；不会对历史 Socket 执行 terminate/unlink。

## 进度记录

- [x] 确认固定全局 Socket 是跨 bundle 互相终止 Agent 的根因。
- [x] 完成 namespace 解析与单元测试。
- [x] 完成构建验证和历史记录；本地提交后等待获准发布新运行时，嵌入式宿主才能升级。

## 决策记录

- 2026-09-01：采用 opt-in namespace，而不是修改全局默认路径或禁用 App Agent 代理，以保留旧调用的兼容性并隔离嵌入式宿主。
