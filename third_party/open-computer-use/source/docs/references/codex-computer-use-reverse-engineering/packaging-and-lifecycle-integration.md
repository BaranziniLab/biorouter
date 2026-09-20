# Packaging And Lifecycle Integration

## 已观察事实

### 1. 官方 computer-use 插件根目录非常薄，只暴露一个 MCP server

`~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/` 当前可见内容主要只有：

- `.codex-plugin/plugin.json`
- `.mcp.json`
- `Codex Computer Use.app`
- `assets/app-icon.png`

没有观察到：

- `.app.json`
- `skills/`
- `hooks.json`

其中 `.mcp.json` 很直接：

```json
{
  "mcpServers": {
    "computer-use": {
      "command": "./Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient",
      "args": ["mcp"],
      "cwd": "."
    }
  }
}
```

这说明对 Codex 插件系统来说，官方 `computer-use` 当前主要是：

- 一个带 UI 元数据的插件包
- 一个通过嵌套 client app 暴露的本地 MCP server

不是“带 skills + app connector + hooks 的复合插件”。

### 2. plugin manifest 暴露的是产品界面信息，不是额外运行时能力

`.codex-plugin/plugin.json` 中可以确认：

- 名称：`computer-use`
- 版本：`1.0.750`
- `mcpServers` 指向 `./.mcp.json`
- `interface.displayName` 为 `Computer Use`
- `interface.brandColor` 为 `#0F172A`
- `interface.defaultPrompt` 内置了 3 个示例 prompt

没有观察到：

- 额外 app manifest 路由
- 额外 skills 注册
- 单独的 hook 文件

这进一步说明官方插件面向宿主暴露的核心集成面仍然是 `mcpServers`。

### 3. 主 app 是菜单栏 / 后台 app，并带自更新能力

`Codex Computer Use.app/Contents/Info.plist` 当前可确认：

- `CFBundleIdentifier = com.openai.sky.CUAService`
- `CFBundleExecutable = SkyComputerUseService`
- `LSUIElement = 1`
- `CFBundleVersion = 750`
- `LSMinimumSystemVersion = 15.0`

同时还能看到 Sparkle 更新字段：

- `SUFeedURL = https://oaisidekickupdates.blob.core.windows.net/mac/cua/alpha/appcast.xml`
- `SUPublicEDKey = 5Yw9jMXMH6O3mJZmpFuQT6ECfC3ZKBfVjWUVMNrElRo=`

这说明官方 service app 不是单纯临时 helper，而是：

- 作为一个独立 macOS app 分发
- 以菜单栏 / agent app 形态后台运行
- 可以通过 Sparkle 独立更新

### 4. bundle 内部有三组资源包，对应不同职责层

主 app 的 `Contents/Resources/` 下当前可见：

- `Package_ComputerUse.bundle`
- `Package_ComputerUseClient.bundle`
- `Package_SlimCore.bundle`

结合 strings，可得到较稳定的职责切分：

- `Package_ComputerUse.bundle`
  - 更偏 service 主体逻辑
  - 包含 `CodexAppServerThreadEventObserver`
  - 包含 `CodexAppServerAuthCache`
  - 包含 `CodexAppServerJSONRPCConnection`
- `Package_ComputerUseClient.bundle`
  - 更偏 MCP client / approval / tool 层
  - 包含 `ComputerUseMCPServer`
  - 包含 `AppApprovalStore`
  - 包含 `ComputerUseIPCClient`
- `Package_SlimCore.bundle`
  - 更偏基础设施与权限 UX
  - 包含 `SystemSettingsAccessCoordinator`
  - 包含 `SystemSettingsAccessoryWindow`
  - 包含 `SystemPermission` / `TCCDialogSystemPermission`

这里的结论仍然基于 strings，而不是源码级确认，但它已经足以支撑一个较清晰的三层划分：

- `ComputerUse`: 宿主集成和核心能力
- `ComputerUseClient`: MCP 暴露和审批状态
- `SlimCore`: 通用权限、系统设置引导和部分基础设施

### 5. `SkyComputerUseClient` 对外 CLI 只公开两个子命令

直接执行：

```text
SkyComputerUseClient --help
```

得到：

```text
USAGE: cua <subcommand>

SUBCOMMANDS:
  mcp
  turn-ended
```

继续查看：

```text
SkyComputerUseClient mcp --help
```

只有：

```text
USAGE: cua mcp
```

而：

```text
SkyComputerUseClient turn-ended --help
```

显示：

```text
USAGE: cua turn-ended [--previous-notify <previous-notify>] <payload>
```

这说明当前正式暴露的 CLI surface 很小：

- `mcp`
  - 启动本地 MCP server
- `turn-ended`
  - 处理 Codex turn 生命周期通知

### 6. client 二进制内嵌了完整 MCP transport 实现，但 CLI 没把它们公开出来

`SkyComputerUseClient` strings 中能看到：

- `mcp.transport.stdio`
- `mcp.transport.http.client`
- `mcp.transport.http.server.stateful`
- `mcp.transport.http.server.stateless`
- `mcp.transport.sse`
- `mcp.transport.in-memory`

以及对应日志：

- `HTTP transport connected`
- `Stateful HTTP server transport started`
- `Stateless HTTP server transport started`
- `Connecting to SSE endpoint`

但结合 `--help` 的结果，目前没有观察到任何 CLI 参数允许：

- 选择 HTTP transport
- 选择 SSE transport
- 打开一个外部可连的 HTTP server

因此当前更稳妥的判断是：

- 这些 transport 来自其内嵌 MCP SDK / 共享库能力
- 不是官方当前对外支持的连接方式

### 7. `turn-ended` 明确接在 Codex 的 legacy notify 生命周期上

本机 `~/.codex/config.toml` 当前包含：

```toml
notify = ["/Users/.../SkyComputerUseClient", "turn-ended"]
```

同时能观察到：

- `SkyComputerUseClient turn-ended --help`
  - 需要 `<payload>`
  - 可选 `--previous-notify`
- `SkyComputerUseService` strings
  - `onTurnEnded`
  - `Codex thread ended or stopped conversationID=%s`
  - `Failed to update Codex Computer Use notify hook: %@`
- `Codex` 宿主 strings
  - `hooks/src/legacy_notify.rs`
  - `legacy notify payload is only supported for after_agent`
  - `agent-turn-complete`
  - `thread-id`
  - `turn-id`
  - `cwd`
  - `client`
  - `input-messages`
  - `last-assistant-message`

这串证据合在一起已经比较明确：

- `turn-ended` 不是给外部用户手工调用的通用命令
- 它是 Codex 宿主在 `after_agent` / `agent-turn-complete` 阶段回调的 lifecycle hook
- `<payload>` 很可能就是 Codex legacy notify 体系下的 after-agent payload
- `--previous-notify` 则很像“保留并串联原有 notify hook”的迁移参数

再补一条来自官方 `codex` 开源源码的对照证据：

- `core/src/config/mod.rs`
  - `notify` 只是一个 argv 数组
- `hooks/src/legacy_notify.rs`
  - Codex 只会把 JSON payload 作为最后一个 argv 参数追加出去
- `hooks/src/registry.rs`
  - `legacy_notify_argv` 直接注册为 `after_agent` hook

开源核心里没有出现：

- `previous-notify`
- “链式 notifier”
- “包装旧 notify hook” 的通用机制

因此当前更强的推断是：

- `--previous-notify` 不是 Codex 核心 hook API 的一部分
- 它更可能是 `SkyComputerUseClient turn-ended` 自己额外引入的兼容参数
- 其目的很可能是：官方 computer-use 在接管 `notify` 配置时，把原有 notifier 通过 `--previous-notify` 保存下来，再由 client 在 turn 结束时选择性继续调用

### 8. `turn-ended` 的 payload 结构现在可以从官方 `codex` 开源源码直接确认

官方 `openai/codex` 仓库里的 `codex-rs/hooks/src/legacy_notify.rs` 当前实现是：

- `notify` 配置会在每次完成一轮 agent turn 后触发
- Codex 会把一个 JSON 字符串作为“最后一个 argv 参数”追加给 notifier
- 该 payload 仅适用于 `after_agent`

源码里的 `UserNotification::AgentTurnComplete` 当前字段为：

```json
{
  "type": "agent-turn-complete",
  "thread-id": "<string>",
  "turn-id": "<string>",
  "cwd": "<string>",
  "client": "<string|null>",
  "input-messages": ["<string>", "..."],
  "last-assistant-message": "<string|null>"
}
```

源码测试里给出的历史兼容 wire shape 也是：

```json
{
  "type": "agent-turn-complete",
  "thread-id": "b5f6c1c2-1111-2222-3333-444455556666",
  "turn-id": "12345",
  "cwd": "/Users/example/project",
  "client": "codex-tui",
  "input-messages": [
    "Rename `foo` to `bar` and update the callsites."
  ],
  "last-assistant-message": "Rename complete and verified `cargo build` succeeds."
}
```

因此现在已经可以把前面的“很可能”收紧成更明确的判断：

- `turn-ended <payload>` 的 `<payload>` 至少和 Codex 现行 `legacy_notify` 的 after-agent JSON wire shape 高度一致
- `type` 的值就是 `agent-turn-complete`
- 这条命令不是一般性的自由表单输入，而是宿主生命周期事件的结构化负载

### 9. 外部 shell 直接执行 `turn-ended` 也会被 launch constraint kill

本机直接执行以下两种命令：

- 合法 JSON payload
- 非法 JSON payload

两者都得到：

```text
status=137
```

也就是子进程收到 `SIGKILL`。

对应最新 crash report 中可见：

- `exception = SIGKILL (Code Signature Invalid)`
- `termination.namespace = CODESIGNING`
- `indicator = Launch Constraint Violation`

这说明从外部 shell 直接调用 `turn-ended` 时，进程在进入可观察的业务层 parse / validate 之前，就已经被系统以 launch constraint 杀掉了。

因此当前还不能通过外部 shell fuzzing 去区分：

- 这个命令是否先 parse payload
- parse 失败会不会给出用户态错误

因为 caller 还没被官方宿主信任链放行。

### 10. 共享 container 当前只看到 analytics，而没看到审批或 auth 主存储

当前 app group container：

- `~/Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/`

可见内容非常少：

- `.com.apple.containermanagerd.metadata.plist`
- `Library/Application Support/Software/Analytics.db`

目前没有在这个 container 里观察到：

- approvals 持久化文件
- auth token 文件
- 显式的 service/client 协调状态文件

这说明到目前为止更像是：

- analytics 确实走 app group 共享
- 审批、turn 生命周期、宿主状态更可能通过进程内或 IPC 传递

### 11. client 有 parent launch constraints，主 app 没观察到同级限制

`codesign -d -r- -vvvv SkyComputerUseClient.app` 可见：

- `Launch Constraints: Has Parent Launch Constraints`
- 资源里有 `SkyComputerUseClient_Parent.coderequirement`
- requirement 当前可读到 `team-identifier = 2DC432GLL2`

而主 app `Codex Computer Use.app` 的同类输出中，目前没看到相同的 parent launch constraint 行。

这和运行时现象一致：

- 外部 `python3` / `node` 直接 pipe 拉起 client 会触发 launch constraint kill
- 长期存活的 client 基本由 OpenAI 签名的 Codex 宿主拉起

### 12. provisioning profile 和实际 container/entitlement 呈现不完全一致

从 `embedded.provisionprofile` 中可读到：

- Team: `OpenAI OpCo, LLC`
- TeamIdentifier: `2DC432GLL2`
- `ProvisionsAllDevices = 1`
- `keychain-access-groups = ["2DC432GLL2.*"]`

同时还可读到 application groups：

- service profile:
  - `group.com.openai.sky.CUAService`
  - `2DC432GLL2.*`
- client profile:
  - `group.com.openai.sky.Service`
  - `group.com.openai.sky.CUAService`
  - `2DC432GLL2.*`

但 `codesign --entitlements :-` 输出里，当前签名 entitlements 呈现的是：

- `2DC432GLL2.com.openai.sky.CUAService`

再对照真实 group container，又看到：

- `~/Library/Group Containers/2DC432GLL2.com.openai.sky.CUAService/`

因此目前能确认的是：

- OpenAI 确实给这组 bundle 配了 application group / keychain group
- 真实可见 container 标识是 `2DC432GLL2.com.openai.sky.CUAService`

但还不能仅根据 provisioning profile 的文本展示，断言所有 group 名称在运行时的精确匹配关系。

## 当前推断

### 1. 官方发布物其实分成三层

更符合现有证据的分层是：

- 插件层
  - `.codex-plugin/plugin.json`
  - 面向 Codex 插件市场与 UI 呈现
- client 层
  - `SkyComputerUseClient mcp`
  - 面向 Codex MCP runtime
- service 层
  - `Codex Computer Use.app`
  - 面向 macOS 权限、桌面 automation 和宿主 lifecycle

开源版如果没有官方宿主，最应该复刻的是：

- client 对外 MCP contract
- service 对本地 automation 的最小能力

而不一定要复刻：

- Sparkle 更新
- 私有插件自安装
- legacy notify 链接法
- 私有 appserver socket

### 2. `turn-ended` 是开源版需要显式重设计的接口，而不是照抄

在官方体系里，`turn-ended` 是：

- 宿主生命周期集成的一部分
- 和 `notify` 配置、旧 hook 串联、per-turn 回收相关

开源版如果没有官方 Codex 宿主，应该把这层改成更透明的方案，例如：

- 显式的 `session/end` MCP 方法
- service 侧超时和清理策略
- 或纯客户端无状态实现

而不是要求用户再去配置一个私有语义的 `notify` hook。

## 当前未决问题

- `--previous-notify` 的精确值格式是什么？是原始命令字符串、argv 序列化结果，还是某种配置引用？
- `AppApprovalStore` 的真实持久化位置在哪里，为什么当前 app group container 里没有明显文件？
- `Package_SlimCore` 除权限 UX 外是否还承载了更多跨产品基础设施？
- client 二进制虽然带了 HTTP/SSE transport，但官方为什么没有公开这些入口，是产品选择还是宿主限制？
