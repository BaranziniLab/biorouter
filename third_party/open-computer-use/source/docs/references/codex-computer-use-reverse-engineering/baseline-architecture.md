# Baseline Architecture

## 已观察事实

### 1. 组件拆分

当前官方 bundle 至少包含两层可执行组件：

- `Codex Computer Use.app`
  - 可执行文件：`SkyComputerUseService`
  - bundle identifier：`com.openai.sky.CUAService`
- `Codex Computer Use.app/Contents/SharedSupport/SkyComputerUseClient.app`
  - 可执行文件：`SkyComputerUseClient`
  - bundle identifier：`com.openai.sky.CUAService.cli`

这说明官方实现不是单一二进制，而是 service app 和 client app 分层。

### 2. Codex 当前如何接入

本机插件配置显示，Codex 不是直接调 service，而是通过内置 client 以 `stdio` 模式拉起 MCP server：

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

结合本机日志，当前集成方式可以确认是：

- server name：`computer-use`
- origin：`stdio`
- transport：`stdio`

### 3. client 的命令行入口

`SkyComputerUseClient --help` 显示它至少有两个子命令：

- `mcp`
  - 说明文字：`Runs the Computer Use client as an MCP server`
- `turn-ended`
  - 说明文字：`Handles a Codex turn-ended notification`

这说明 client 不只是 MCP 包装层，还显式参与了“turn 结束”这类会话生命周期事件。

本机 `~/.codex/config.toml` 里还能看到：

```toml
notify = ["/Users/.../SkyComputerUseClient", "turn-ended"]
```

这说明 Codex 在本机确实把 `turn-ended` 当作全局通知入口来调用，而不是一个未使用的调试命令。

### 4. service / client 共享安全边界

service 和 client 共享同一个 application group：

- `2DC432GLL2.com.openai.sky.CUAService`

两者 entitlement 中都出现：

- `com.apple.security.application-groups`
- `com.apple.security.automation.apple-events`

这说明它们属于同一安全边界，预计会共享持久化状态、权限状态或 IPC 配置。

### 5. service 二进制暴露出的能力面

`SkyComputerUseService` 中能稳定观察到以下能力模块：

- 权限与引导：
  - `CUAServicePermissionState`
  - `CUAServicePermissionsWindow`
  - `SystemSettingsAccessoryWindow`
- Accessibility / UI 树：
  - `AXNotificationObserver`
  - `SystemFocusedUIElementObserver`
  - `KeyWindowTracker`
  - `WindowOrderingObserver`
- 屏幕截图与窗口层：
  - `ScreenCaptureKit`
  - `SCScreenshotManager`
  - `SCShareableContent`
  - `WindowBoundsObserver`
- 输入与交互：
  - `EventTap`
  - `clickEventTap`
  - `keyboardEventTap`
  - `drag`
  - `scroll`
- 可视化层：
  - `ComputerUseCursor`
  - `FogCursorStyle`
  - `virtualCursor`
- MCP 支持：
  - `MCP/Server.swift`
  - `MCP/StdioTransport.swift`
  - `MCP/StatefulHTTPServerTransport.swift`
  - `MCP/StatelessHTTPServerTransport.swift`
  - `MCP/SSEClientTransport.swift`

### 6. 当前公开可见的 tools

strings 和运行时都指向同一组 tools：

- `list_apps`
- `get_app_state`
- `click`
- `perform_secondary_action`
- `scroll`
- `drag`
- `type_text`
- `press_key`
- `set_value`

### 7. 当前会话可见的 tool schema

下面这组 schema 以当前 Codex 会话真实暴露的 MCP tool 定义为准，并结合一次运行时调用结果补充使用语义。

#### `list_apps`

```json
{}
```

- 无参数。
- 返回当前机器上正在运行或近 14 天使用过的 app 列表。
- 返回文本中会包含 app 名、bundle identifier、`running` 状态、`last-used` 和 `uses`。

#### `get_app_state`

```json
{
  "app": "string"
}
```

- `app`：app 名或 bundle identifier。
- 作用是启动或复用 app use session，并返回当前主窗口状态。
- 运行时返回至少包含：
  - app 标识和 pid
  - 窗口层级和 accessibility tree
  - 每个 element 的索引号
  - element 的 role、value、description、`settable` 等属性
  - `Secondary Actions`
  - 当前窗口截图

#### `click`

```json
{
  "app": "string",
  "element_index": "string?",
  "x": "number?",
  "y": "number?",
  "click_count": "integer? = 1",
  "mouse_button": "\"left\" | \"right\" | \"middle\" ? = \"left\""
}
```

- `app`：目标 app。
- `element_index`：按 accessibility element 定位点击目标。
- `x` / `y`：按截图像素坐标点击目标。
- `click_count`：点击次数，默认 `1`，可用于 double-click 或 triple-click。
- `mouse_button`：鼠标按键，默认 `left`。
- 接口语义上，`element_index` 和 `x` / `y` 是两套寻址方式，通常应二选一。

#### `perform_secondary_action`

```json
{
  "app": "string",
  "element_index": "string",
  "action": "string"
}
```

- `app`：目标 app。
- `element_index`：目标 accessibility element。
- `action`：element 当前暴露的 secondary action 名称。
- `action` 不是固定枚举，必须来自 `get_app_state` 输出中的 `Secondary Actions`。

#### `scroll`

```json
{
  "app": "string",
  "direction": "string",
  "element_index": "string",
  "pages": "number? = 1"
}
```

- `app`：目标 app。
- `direction`：工具说明约束为 `up` / `down` / `left` / `right`。
- `element_index`：必须是可滚动的 element。
- `pages`：滚动页数，默认 `1`；官方 `1.0.755` 的 tool schema 已改为 `number`，支持小数页数。
- 这是 element-scoped scroll，不是全局屏幕滚动。

#### `drag`

```json
{
  "app": "string",
  "from_x": "number",
  "from_y": "number",
  "to_x": "number",
  "to_y": "number"
}
```

- `app`：目标 app。
- `from_x` / `from_y`：拖拽起点像素坐标。
- `to_x` / `to_y`：拖拽终点像素坐标。
- 当前公开接口里，拖拽只支持坐标，不支持 `element_index`。

#### `type_text`

```json
{
  "app": "string",
  "text": "string"
}
```

- `app`：目标 app。
- `text`：要输入的字面文本。
- 更适合普通文本录入，不负责表达快捷键语义。

#### `press_key`

```json
{
  "app": "string",
  "key": "string"
}
```

- `app`：目标 app。
- `key`：按键或组合键，采用 `xdotool key` 风格。
- 工具说明示例包括：
  - `a`
  - `Return`
  - `Tab`
  - `super+c`
  - `Up`
  - `KP_0`
- 当前 `1.0.755` binary 里还能看到 `BackSpace`、`Page_Up`、`Prior`、`Next`、`F1...F12`、`KP_0...KP_9`、`KP_Enter` 等 key table 字符串；开源版 parser 已按这些常用 xdotool alias 收敛。

#### `set_value`

```json
{
  "app": "string",
  "element_index": "string",
  "value": "string"
}
```

- `app`：目标 app。
- `element_index`：目标 settable element。
- `value`：要直接写入的值，当前 schema 中统一为字符串。
- 这是比 `type_text` 更语义化的输入方式，适合 search field、text field 等可直接赋值控件。

## 当前推断

### 1. 官方实现的最小分层

当前最合理的分层判断是：

- `SkyComputerUseService`
  - 权限管理、状态栏、窗口/光标 overlay、系统集成、Accessibility、截图、审批与会话状态
  - 还负责和 Codex appserver 的宿主 IPC、notify hook 和 plugin 生命周期集成
- `SkyComputerUseClient`
  - MCP 入口、turn 生命周期桥接、和 service 的本地通信

### 2. transport 能力和当前启用状态不是一回事

虽然 service / client 二进制中能看到 HTTP、SSE、network 相关 MCP transport 符号，但当前安装包的真实启用方式仍然只有 `stdio`。开源版设计时不能直接假设官方对外公开了 HTTP/SSE server。

补充一点：当前 Node MCP SDK 的 `stdio` framing 是 newline-delimited JSON，而不是 `Content-Length`。这意味着开源版如果要优先兼容主流 Node client，最直接的做法也是先把 JSON line `stdio` 路径打稳。

再补一层：官方实现里不只是“stdio 启一个本地 server”这么简单。`SkyComputerUseService` 还会主动连接 Codex 宿主维护的 `codex-ipc` Unix socket，并在里面处理 thread-end、auth 和 plugin 集成。这说明开源版如果没有官方宿主，也应该明确删掉这类私有 appserver 依赖，而不是半复制一个绑定宿主的结构。

### 3. 开源版不必复制官方的所有产品壳

从能力上看，真正必须复现的是：

- app 发现
- 窗口截图
- Accessibility 树读取
- 鼠标键盘动作
- 权限引导
- 会话状态和审批模型

### 4. 当前 tool 面是一个以 Accessibility tree 为中心的最小接口层

当前这组公开 tools 可以压缩成三层：

- 发现：
  - `list_apps`
- 读状态：
  - `get_app_state`
- 做动作：
  - `click`
  - `perform_secondary_action`
  - `scroll`
  - `drag`
  - `type_text`
  - `press_key`
  - `set_value`

这说明官方当前暴露的是一套很小的 automation kernel，而不是完整的桌面控制 API。

### 5. `element_index` 是第一公民，坐标只是补充定位方式

大部分交互类工具都围绕 `get_app_state` 产出的 accessibility tree 工作：

- `perform_secondary_action`
- `scroll`
- `set_value`
- `click` 的主要模式

只有少数动作明显是纯几何操作：

- `drag`
- `click` 的 `x` / `y` 模式

这意味着官方实现优先依赖 AX 语义定位，而不是把 screenshot 当主导航面。

### 6. 开源兼容层最好保留聚合型 `get_app_state`

当前公开接口没有单独的：

- `launch_app`
- `screenshot`
- `get_ax_tree`
- `wait`
- `hover`

而是把“必要时拉起 session + 读取窗口树 + 读取截图”合并到了 `get_app_state`。如果开源版目标之一是兼容现有 agent 使用习惯，保留这个聚合入口会更稳。

状态栏、虚拟光标、PIP 一类的产品壳可以晚于核心 automation path。

## 对开源版的直接启发

- MCP server 可以单独设计成公开、稳定、可被任意 client 拉起的入口，不必复用官方那种宿主绑定方式。
- service/client 的边界应该尽早显式化，否则后续很容易被私有宿主约束反噬。
- transport、权限、Automation 内核、UI/overlay 最好拆成独立模块，避免逆向时看到的多层耦合直接复制进开源实现。
- 开源版 schema 可以直接对齐这 9 个 tools 作为兼容层第一版，再在实现内部拆成 app discovery、AX snapshot、screen capture、input dispatcher 和 approval/session manager。
