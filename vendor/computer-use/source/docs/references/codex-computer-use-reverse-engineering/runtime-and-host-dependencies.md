# Runtime And Host Dependencies

## 已观察事实

### 1. `SkyComputerUseClient mcp` 不是任意外部 client 都能直连

在本机上直接执行：

```text
SkyComputerUseClient mcp
```

如果直接用一次性 shell 命令执行，它可能很快退出；但在 PTY 下保持 `stdin` 打开时，进程可以持续存活并等待输入。

这说明：

- `mcp` 子命令真实存在。
- 单看“一次性 shell 执行很快结束”不能直接推出宿主校验失败，其中有一部分只是 `stdin`/EOF 行为。

### 2. Inspector 直连的失败模式

用 MCP Inspector 以 `stdio` 方式拉起时，出现过两类失败：

- 参数填错时：
  - `spawn -y ENOENT`
  - 这是 Inspector 配置错误，不是 server 本身问题。
- 命令改对后：
  - Inspector 建好了 stdio client/server transport
  - 随后对方很快断开，Inspector 侧报 `write EPIPE`

`EPIPE` 在这里说明 child process 已经退出，Inspector 往其 stdin 写入初始化消息时管道已断。

### 3. 外部 pipe/stdin 拉起会触发 codesigning kill

在本机上分别用以下方式拉起：

- `node` 的 `child_process.spawn(..., stdio: ['pipe', 'pipe', 'pipe'])`
- `python3` 的 `subprocess.Popen(..., stdin=PIPE, stdout=PIPE, stderr=PIPE)`
- Python MCP SDK 的 `stdio_client(...)`

都能稳定复现：

- 子进程很快收到 `SIGKILL`
- 父进程侧看到 `Broken pipe` 或 `EPIPE`
- Python MCP SDK 路径上，`stdio_client` 先成功进入 `spawn`，但在 `ClientSession.initialize()` 写入阶段抛出 `anyio.BrokenResourceError`

与之对照：

- 在 PTY 下直接运行 `SkyComputerUseClient mcp`，进程可以常驻等待输入

这说明当前 bundle 至少区分了：

- 交互式终端拉起
- 外部进程经由 pipe/stdin 的自动化拉起

### 4. Inspector / Node / Python SDK 尝试都会留下 crash report

`~/Library/Logs/DiagnosticReports/` 中可见多份 `SkyComputerUseClient-2026-04-17-1142xx.ips`，这些 crash report 的共同特征是：

- `parentProc`: `node` 或 `python3`
- `exception`: `SIGKILL (Code Signature Invalid)`
- `termination.namespace`: `CODESIGNING`
- `indicator`: `Launch Constraint Violation`

目前只能确认：

- 某些通过外部自动化 caller 以 pipe/stdin 拉起的 client 进程会被系统以 launch constraint violation 杀掉。
- Python MCP SDK 的最小复现并没有绕过这个约束，它和 Node / Inspector 的根因一致。

目前还不能确认：

- 这是否是“外部 caller 无法直连”的唯一原因。

原因是 PTY 场景下 client 可以正常存活，而 crash report 主要和 pipe/stdin 自动化拉起路径绑定。

### 5. 能稳定存活的 client 都挂在 OpenAI 签名的 Codex 进程下面

当前机器上长期存活的 `SkyComputerUseClient mcp` 进程，其父进程主要有两类：

- `/Applications/Codex.app/Contents/Resources/codex app-server --analytics-default-enabled`
- Homebrew 安装的 `codex` 二进制：
  - `/opt/homebrew/lib/node_modules/@openai/codex/.../codex`

这两类父进程的共同点是：

- TeamIdentifier 都是 `2DC432GLL2`

与之对照：

- `python3` 作为父进程时，client 会因 `Launch Constraint Violation` 被系统 kill。

这进一步支持当前判断：

- parent launch constraint 很可能不是泛化意义上的“任意 shell/任意本地 caller”，而是要求父进程满足 OpenAI 自家的签名边界。

### 6. client 明确包含宿主 / 认证相关字符串

`SkyComputerUseClient` 的 strings 中可以直接看到：

- `Sender process is not authenticated`
- `Could not find Service app`
- `sessionApprovedBundleIdentifiers`
- `approvedBundleIdentifiers`
- `ComputerUseIPCClient`
- `ComputerUseIPCRequest`
- `ComputerUseIPCAppState`
- `ComputerUseIPCAppPerformActionRequest`

这些字符串说明 client 至少显式考虑了：

- 发送方身份认证
- service app 的定位
- 基于 session 的 app 审批状态
- 本地 IPC 请求模型

### 7. service 明确连接到 Codex appserver 的 Unix socket

当官方 `computer-use` tool 被真实调用并把 service 拉起后，可以直接观察到：

- `SkyComputerUseService` 打开了一个匿名 Unix socket FD
- 对端是 `Codex.app` 进程持有的：
  - `/var/folders/.../T/codex-ipc/ipc-501.sock`

本机 `lsof` 已确认：

- `SkyComputerUseService` 的 Unix socket peer 指向 `Codex.app`
- `Codex.app` 自身持有并监听 `codex-ipc/ipc-501.sock`

这是当前最直接的本地 IPC 证据之一。

### 8. service 二进制里已经明确出现 Codex appserver IPC 语义

`SkyComputerUseService` strings 中直接包含：

- `CodexAppServerThreadEventObserver`
- `CodexAppServerJSONRPCConnection`
- `CodexAppServerAuthCache`
- `CodexAppServerAuthProvider`
- `Connected to Codex appserver IPC socket at %s`
- `Failed to connect to Codex appserver IPC socket:`
- `Codex appserver IPC connection closed before a complete frame was read`
- `Codex thread ended or stopped conversationID=%s`

这说明 service 和 Codex 宿主之间不是松散关系，而是明确存在一个 appserver IPC / JSON-RPC 集成层。

### 9. service 里还能看到 plugin / notify hook 自安装痕迹

`SkyComputerUseService` strings 里还能看到：

- `Failed to update Codex Computer Use notify hook: %@`
- `Failed to install Codex Computer Use plugin: %@`
- `Skipping Codex Computer Use self-install for plugin-managed app.`
- `Couldn't find running Codex app, falling back to using Codex CLI in PATH via /usr/bin/env`
- `Found codex CLI executable in running Codex application at %{public}s`

这说明 service 至少承担了：

- 向 Codex 宿主安装 / 更新 plugin
- 更新 `turn-ended` notify hook
- 在 `Codex.app` 和 `codex` CLI 两种宿主之间做发现和回退

### 10. 官方 MCP stdio framing 是 JSON line

从本机 Inspector 附带的 `@modelcontextprotocol/sdk` 可以确认，当前 Node SDK 的 stdio transport 使用的是一行一个 JSON-RPC message：

```js
export function serializeMessage(message) {
  return JSON.stringify(message) + '\\n';
}
```

也就是说，当前官方 Node SDK 的 stdio MCP 不是 `Content-Length` framing，而是 newline-delimited JSON。

这条结论曾通过一次临时 Python 复现实验进一步钉住。实验路径是：

1. 用 Python `mcp` SDK 的 `stdio_client` 拉起 `SkyComputerUseClient mcp`
2. `stdio_client` 先成功进入 `spawn`
3. `ClientSession.initialize()` 写入时发生 `BrokenResourceError`
4. 系统侧生成新的 `SkyComputerUseClient-*.ips`，终止原因仍为 `Launch Constraint Violation`

后续清理仓库时，这个一次性 Python / `uv` 探针已经移除，因为该仓库不维护 Python 运行链路；这里保留的是实验结论，而不是当前可执行入口。

### 11. service / client 共享 application group

两者 entitlement 中都声明了同一个 application group：

- `2DC432GLL2.com.openai.sky.CUAService`

本机对应 group container 中当前可见内容包括：

- `~/Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/Library/Application Support/Software/Analytics.db`

### 12. shared analytics 能看到 launch 事件

`Analytics.db` 中已经观察到这些事件：

- `$set`
- `cua_service_launched`
- `computer_use_mcp_server_launched`
- `cua_service_idle_timeout_reached`

这说明：

- service 和 client 至少共享一套 analytics/持久化层。
- service 存在 idle timeout 概念，不是永久前台常驻工作模式。
- 至少这次 Python SDK 复现实验没有留下新的 `computer_use_mcp_server_launched` 事件，说明它可能在 client 完成自报启动前就已被系统 kill。

### 13. `turn-ended` 很可能走独立 IPC 请求

`SkyComputerUseClient` 的 strings 里能看到：

- `ComputerUseIPCCodexTurnEndedRequest`
- `ComputerUseCodexTurnEndedCommand`
- `CodexTurnEndedNotification`

再结合 `~/.codex/config.toml` 的：

```toml
notify = ["/Users/.../SkyComputerUseClient", "turn-ended"]
```

当前可以确认：

- `turn-ended` 不是无意义的占位命令。
- client 至少为“Codex 一轮结束”定义了单独的 IPC 请求模型。

### 14. client 自身带有 parent launch constraint

`codesign -d --verbose=5 SkyComputerUseClient.app` 能直接看到：

- `Launch Constraints:`
  - `Has Parent Launch Constraints`

同时 bundle 中还存在：

- `Contents/Resources/SkyComputerUseClient_Parent.coderequirement`

该文件当前是一个 plist，至少声明了：

- `team-identifier = 2DC432GLL2`

这说明外部 caller 受限并不是纯应用层策略，而是 bundle 签名 / launch constraint 的一部分。

### 15. 外部 shell 直接执行 `turn-ended` 也同样会触发 launch constraint kill

本机直接从 shell 执行：

- `SkyComputerUseClient turn-ended '<valid-json>'`
- `SkyComputerUseClient turn-ended '{bad-json}'`

两者都返回：

```text
status=137
```

同时最新 crash report 中可见：

- `exception`: `SIGKILL (Code Signature Invalid)`
- `termination.namespace`: `CODESIGNING`
- `indicator`: `Launch Constraint Violation`

这说明：

- 受限的不只是 `mcp` 子命令
- `turn-ended` 这个 lifecycle 子命令在外部 shell 场景下也走同一套受信 caller 限制

因此现在不能把 `turn-ended` 当成“对外可独立调用的调试入口”。

### 16. 目前还没有看到公开暴露的 socket / launchd service

当前检查结果里：

- 没发现 `SkyComputerUseService` 对外监听 TCP 端口。
- 没发现稳定可见的 Unix socket 暴露给外部 client。
- `launchctl print gui/<uid>/com.openai.sky.CUAService` 没找到同名公开 service。

这说明如果 client 和 service 之间存在本地通信，更可能是：

- 私有 XPC / app group / bundle 发现机制
- 或其他不直接暴露给任意外部进程的宿主内通信

### 17. `1.0.755` 对 raw app-server helper 增加了 service-side sender authorization

2026-04-21 复查官方 bundled `computer-use` `1.0.755` 时，同一台机器上出现了新的分叉行为：

- `scripts/computer-use-cli` 的 `app-server` 模式仍然可以通过 `mcpServerStatus/list` 看到官方 `computer-use` 的 9 个 tools。
- 但从外部 shell 启动一个临时 `codex app-server`，再调用 `mcpServer/tool/call list_apps`，返回：

```text
Apple event error -10000: Sender process is not authenticated
```

系统日志把这次失败分成了两段：

- `SkyComputerUseClient` 通过 Apple Events 向 `SkyComputerUseService` 发送 `SkCu/SndR` 请求。
- TCC 对这次 Apple Events 请求返回 `ACCESS GRANTED`。
- 随后 service 侧出现多次签名 / trust 校验 activity，并输出 `Computer Use` 分类下的错误日志。
- 失败路径没有出现 `Tracking Computer Use IPC client process ...`。

对照同一轮里正常 Codex agent/tool 调用官方 `computer-use`：

- `list_apps` 可以成功返回。
- service 会先启动 `Codex AppServer Thread Events` observer，并连接到 `codex-ipc` socket。
- service 随后记录 `Tracking Computer Use IPC client process ...`。

所以当前更准确的判断是：

- `codex app-server` 这个二进制由 OpenAI 签名，只能满足 `SkyComputerUseClient` 的 parent launch constraint。
- 满足 parent launch constraint 并不等于 tool call 已经通过官方 Computer Use 的 sender authorization。
- 官方 service 还要求请求来自它能认证并追踪的 Codex/ComputerUse IPC client；外部 helper 临时创建的 raw app-server thread 现在不再能稳定复用这条私有链路。

## 当前推断

### 1. client 很可能不是独立产品边界，而是宿主绑定的桥接层

目前最符合证据的判断是：

- `SkyComputerUseClient` 负责把本地 computer-use 能力包装成 MCP
- 但它真正依赖 `Codex Computer Use.app` 或受信宿主上下文
- 因此它不像公开的通用 MCP server 那样允许任意外部 caller 直接复用

### 2. 外部直连失败至少有两层风险

当前观察到的风险至少有两类：

- macOS launch constraint 直接拦截外部自动化拉起
  - 证据：`CODESIGNING / Launch Constraint Violation`
  - 证据：`Has Parent Launch Constraints`
- service-side sender authorization 拒绝 raw helper 发起的工具调用
  - 证据：Apple Events/TCC 已 `ACCESS GRANTED` 后仍返回 `Sender process is not authenticated`
  - 证据：成功路径会记录 active Computer Use IPC client，失败路径不会
- 宿主集成层内部还依赖 Codex appserver / auth / plugin lifecycle
  - 证据：`CodexAppServerJSONRPCConnection`
  - 证据：`CodexAppServerAuthProvider`
  - 证据：`Connected to Codex appserver IPC socket at %s`
- 宿主认证 / service 定位失败
  - 证据：`Sender process is not authenticated`、`Could not find Service app`

这两类问题目前不能合并成一个根因。

### 3. 官方实现里存在“session 级 app 审批”

`sessionApprovedBundleIdentifiers` 这类字符串表明，官方实现不只是拿到系统级 Accessibility/Screen Recording 权限就全量控制所有 app，还维护了一层更细的 session 级审批状态。

对开源版来说，这是个重要产品边界：

- 是否允许某个 app 被 computer-use 控制
- 这个允许是一次性、会话级还是永久级

应该成为显式设计，而不是散落在代码里。

## 当前未决问题

- service 和 client 之间除 appserver socket 外，到底还使用 XPC、app group 文件协调，还是别的 IPC 机制？
- `SkyComputerUseClient_Parent.coderequirement` 里的 parent launch constraint 除 team identifier 外还有没有更细的约束来源？
- `turn-ended` 子命令除了会话清理，还会不会参与审批状态同步、idle timeout 刷新或后台 app 回收？
- `approvedBundleIdentifiers` 的持久化位置在哪里，是否只保存在进程内？
- 官方是否有计划暴露受支持的本地调试入口，让外部工具在不复用私有 sender authorization 的情况下探测 bundled `computer-use`？
- `codex-ipc/ipc-501.sock` 上跑的具体 JSON-RPC 方法和认证过程是什么？

## 下一批分析建议

- 如需继续复现实验，优先现写最小临时 stdio client，而不是依赖 Inspector 或把一次性探针长期留在仓库里。
- 继续查 service / client 的 XPC、mach service、bundle lookup、app group 读写痕迹。
- 针对 `turn-ended` 子命令补一轮 strings / 运行时分析。
- 对 analytics / preferences / group container 做更细的时间线比对，观察一次真实 Codex 会话会写入哪些审批和生命周期事件。
