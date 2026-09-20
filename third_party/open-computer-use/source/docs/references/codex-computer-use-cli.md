# `scripts/computer-use-cli`

这个目录下放的是一个独立 Go CLI，用来做两类事情：

- 探测官方 Codex 桌面版自带的闭源 `computer-use`
- 直接连接普通 stdio MCP server，例如本仓库产出的 `open-computer-use`

它不是仓库主产物，而是调试/逆向分析辅助工具。

## 为什么需要它

在这台机器上，官方 bundled `computer-use` 的可执行文件 `SkyComputerUseClient` 不能被一个普通 unsigned MCP client 稳定直接拉起。

已验证的现象是：

- 标准 stdio MCP client 握手前就退出
- 官方 Go MCP SDK 自带示例也会失败
- crash report 指向 `Launch Constraint Violation` / `CODESIGNING`

结论是：这不是单纯的 MCP 协议兼容问题，而是宿主签名/父进程约束，以及官方 service 自己的 sender authorization。要探测官方 bundled `computer-use` 的工具清单，可以借助已签名的 Codex 宿主，通过 `codex app-server` 走 `mcpServerStatus/list`；真实 tool call 不应再依赖这个 raw helper。

## 目录位置

```text
scripts/computer-use-cli/
```

目录本身是一个独立 Go module，内部自带 `go.mod`、单测和 README。

## AI 默认用法

如果任务目标是“验证官方 Codex 自带的 `computer-use` 能不能列工具”，优先这样跑：

```bash
cd scripts/computer-use-cli
go run . list-tools --transport app-server
```

如果任务目标是验证可脚本化 tool call，优先指向本仓库的 `open-computer-use` direct server。

默认 `auto` 模式会自动选择：

- 官方 bundled `computer-use` -> `app-server`，当前只适合作为工具清单探测路径
- 显式传入的非 Sky server binary -> `direct`

本地兼容性测试默认优先解析 `~/.codex/plugins/computer-use` 里的非 translocated
`1.0.750` 旧安装根，并检查它的 plugin manifest version。没有这份旧根时再回退到
`~/.codex/plugins/cache/.../1.0.750`，最后回退到最新安装版本。需要对比其他版本时显式传：

```bash
COMPUTER_USE_PLUGIN_VERSION=1.0.755 go run . resolve-server
go run . resolve-server --plugin-version latest
go run . list-tools --transport app-server --plugin-version host
```

`--plugin-root` / `--server-bin` 的优先级高于版本选择。版本选择只影响
`resolve-server`、direct launch 的目标路径，以及传给 `codex app-server` 的临时
`mcp_servers."computer-use"` 覆盖。需要完全使用 Codex host 自己的 MCP 配置时，
传 `--plugin-version host`。Codex CLI 的 `-c` override 只按 `.` 拆 key，不解析 quoted
dotted key，所以实现里实际使用的是 `mcp_servers.computer-use.*`。

## 两种 transport

### 1. `app-server`

适用于官方 bundled `computer-use`。

```bash
cd scripts/computer-use-cli
go run . list-tools --transport app-server
```

这个模式会：

1. 启动 `codex app-server`
2. 建一个 ephemeral thread
3. 通过 `mcpServer/tool/call` 调目标 server

截至官方 bundled `computer-use` `1.0.755`，这条 raw helper 路径只能继续稳定用于工具清单探测；实际 `mcpServer/tool/call` 可能返回：

```text
Apple event error -10000: Sender process is not authenticated
```

复查日志显示 Apple Events/TCC 已经接受请求，随后 `SkyComputerUseService` 在自己的 sender authorization / active IPC client 追踪层拒绝了调用。因此它不是一个可依赖的“外部直接调用官方 computer-use”的通用入口。需要真实调用官方工具时，优先使用正常 Codex agent/tool 调用链；需要可脚本化直连时，使用本仓库的 `open-computer-use` direct 模式。

本地兼容性测试会默认把 app-server 的 `computer-use` MCP 配置临时覆盖到 `1.0.750`。
在当前工作区，cache 里的 `1.0.750` 带 `com.apple.quarantine`，LaunchServices 会把
service AppTranslocation，`call list_apps` 会返回 `Apple event error -1708: Unknown error`；
非 translocated 的 `~/.codex/plugins/computer-use` 旧安装根可以正常调用 `list_apps`。

如果本机 Codex 可执行不在默认位置，可以显式指定：

```bash
CODEX_APP_SERVER_BIN=/Applications/Codex.app/Contents/Resources/codex \
go run . list-tools --transport app-server
```

### 2. `direct`

适用于普通 stdio MCP server，例如本仓库本地产出的 `open-computer-use`。

```bash
cd scripts/computer-use-cli
go run . call list_apps \
  --transport direct \
  --server-bin ~/.codex/plugins/cache/open-computer-use-local/open-computer-use/0.1.7/scripts/launch-open-computer-use.sh
```

## 什么时候不要再重试 direct

如果目标是官方 `SkyComputerUseClient`，而且你已经看到下面这类现象，就不要再浪费时间试“换一个通用 MCP client”：

- `EOF`
- `broken pipe`
- 初始化前退出
- crash report 里出现 `Launch Constraint Violation`

这类情况下，优先切回 `app-server` 模式。

## 本地验证

```bash
cd scripts/computer-use-cli
go test ./...
```

如果只是想快速确认工具链没坏，最小正向验证通常是：

```bash
cd scripts/computer-use-cli
go run . list-tools
go run . call list_apps --transport direct --server-bin /path/to/open-computer-use
```
