# Internal IPC Surface

## 已观察事实

### 1. service 和 client 之间至少存在一套单独的 ComputerUse IPC 协议

在 `SkyComputerUseService` 和 `SkyComputerUseClient` 两个二进制里，都能稳定看到同一组类型名：

- `ComputerUseIPCClient`
- `ComputerUseIPCServer`
- `ComputerUseIPCRequest`
- `ComputerUseIPCEmptyResponse`
- `ComputerUseIPCApp`
- `ComputerUseIPCAppState`
- `ComputerUseIPCDiscoveredApp`

这说明除了：

- client 对外的 MCP 协议
- service 对 Codex appserver 的宿主 IPC

之外，官方实现内部还单独维护了一层 `ComputerUseIPC*` 协议，负责 client 和 service 之间的本地调用。

### 2. 内部 IPC request 类型已经可以从符号名里拼出一版能力表

当前在两个二进制里都能看到的 request 类型包括：

- `ComputerUseIPCListAppsRequest`
- `ComputerUseIPCAppStartRequest`
- `ComputerUseIPCAppModifyRequest`
- `ComputerUseIPCAppPerformActionRequest`
- `ComputerUseIPCAppGetSkyshotRequest`
- `ComputerUseIPCCodexTurnEndedRequest`
- `ComputerUseIPCAppUsageRequest`

再结合字段名和 MCP tool 行为，当前最稳妥的映射是：

- `ListAppsRequest`
  - 对应外部 `list_apps`
- `AppGetSkyshotRequest`
  - 对应外部 `get_app_state`
  - `Skyshot` 很像“截图 + AX tree + 结构化状态”的内部表示
- `AppPerformActionRequest`
  - 对应 `click` / `perform_secondary_action` / `set_value` / `scroll` / `drag` / `press_key` / `type_text`
- `AppModifyRequest`
  - 至少包含 `activate` / `deactivate`
- `CodexTurnEndedRequest`
  - 对应 `turn-ended` 生命周期回调

### 3. `get_app_state` 在内部大概率不是“直接返回 app state”，而是“取一份 skyshot”

当前 strings 中同时出现：

- `ComputerUseIPCAppState`
- `ComputerUseIPCSkyshot`
- `ComputerUseIPCSkyshotResult`
- `ComputerUseIPCAppGetSkyshotRequest`
- `SkyshotCapture`
- `RefetchableSkyshotAXTree`
- `SkyshotClassifier`

这说明官方实现内部更像是：

1. 先对目标 app 取一份 `Skyshot`
2. 再从 `Skyshot` 里构建截图、窗口和 AX tree
3. 最后由 client 组装成对 MCP 层暴露的 `get_app_state` 返回值

这里的关键点是：

- 外部 MCP API 叫 `get_app_state`
- 但内部的核心抽象更接近 `skyshot`

这对开源版是个重要提示：

- 如果要复刻官方行为，核心内部模型可能不该直接叫 `AppState`
- 更合理的是“截图采集结果 + AX 树 + 目标 app 元数据”的复合对象

### 4. action 执行内部统一走 `AppPerformActionRequest`

当前可见的字段和 coding keys 包括：

- `ComputerUseIPCAction`
- `ComputerUseIPCLocationSpecifier`
- `CoordinateCodingKeys`
- `ElementIDCodingKeys`
- `ClickCodingKeys`
- `PerformSecondaryActionCodingKeys`
- `SetValueCodingKeys`
- `ScrollCodingKeys`
- `DragCodingKeys`
- `PressKeyCodingKeys`
- `TypeCodingKeys`

同时还能看到字段名：

- `coordinate`
- `elementID`
- `action`
- `text`
- `mouseButton`
- `clickCount`

这说明内部 action 层大概率已经统一成：

- 一个 action 枚举
- 一个 location specifier
  - 坐标模式
  - 元素 ID 模式
- 若干具体 action payload

也就是说，外部 7 个动作型 tools 在内部未必是一组完全分离的方法，更像是同一条 `performAction` 通道上的不同变体。

### 5. app 生命周期内部和工具动作是分开的

当前还能看到：

- `ComputerUseIPCAppStartRequest`
- `ComputerUseIPCAppModifyRequest`
- `Modification`
- `ActivateCodingKeys`
- `DeactivateCodingKeys`
- `active`
- `currentApp`
- `isRunning`

这说明官方内部把下面两类操作拆开了：

- app 级生命周期
  - 启动
  - 激活
  - 取消激活
- UI action
  - click / type / scroll / drag / ...

这和外部 MCP 面不完全一致，因为外部当前没有公开：

- `activate_app`
- `deactivate_app`
- `start_app`

但 service 内部显然保留了这层能力。

### 6. sender authorization 是内部 IPC 的显式组成部分

`SkyComputerUseService` strings 里可见：

- `ComputerUseIPCSenderAuthorization`
- `ProcessIdentity`
- `CodeSignature`
- `Requirement`
- `SecurityError`
- `Sender process is not authenticated`

这说明 client-service 之间的内部 IPC 并不是“谁都能连上的本地 socket 调用”，而是显式校验发送方身份的。

结合前面已确认的事实：

- client 本身有 parent launch constraints
- 外部 `python3` / `node` caller 会被系统 kill

当前更合理的判断是：

- 发送方认证不仅发生在 macOS launch constraint 层
- service 自己的内部 IPC 层也有一层 sender authorization / code signature requirement 校验

### 7. system permission gating 也是内部 IPC request 的一部分

`SkyComputerUseService` strings 里还能看到：

- `ComputerUseIPCPermissionResult`
- `ComputerUseIPCRequestRequiringSystemPermissions`
- `ensureApplicationHasPermissions`
- `Failed to request access to permission: %@`
- `Failed to open System Settings for permission: %@`

这说明内部 IPC 请求里已经分化出一类：

- 需要系统权限的请求

也就是说，官方实现不是单纯在外层 UI 里判断“有没有 Accessibility / Screen Recording 权限”，而是把权限需求并入了内部 request pipeline。

### 8. service 会跟踪 active IPC client，并在为空时自杀

`SkyComputerUseService` strings 里直接有：

- `No active Computer Use IPC client processes; terminating service`
- `Codex Computer Use idle timeout reached; terminating service`

这说明 service 的生命周期不是长期 daemon，而是至少同时受两类条件控制：

- 是否还有活跃的 IPC client
- 是否触发 idle timeout

因此官方 service 更像：

- on-demand 启动
- 有使用时保活
- 无人连接或空闲超时后自回收

### 9. service 和 client 都弱链接了 `libswiftXPC`

`otool -L` 能看到：

- `libswiftXPC.dylib (weak)`

但目前还没有观察到：

- 明确的 mach service 名
- `launchd` 服务注册名
- 可直接枚举到的公开 XPC endpoint

所以当前只能确认：

- 两者具备使用 Swift XPC 的依赖条件

不能确认：

- 当前 `ComputerUseIPC*` 是否真的建立在 XPC 之上

### 10. app 使用行为和审批状态有独立对象

当前还能看到：

- `ComputerUseIPCAppUsageRequest`
- `AppApprovalStore`
- `sessionApprovedBundleIdentifiers`
- `approvedBundleIdentifiers`
- `bundleIdentifiersWithDeliveredInstructions`

这说明内部至少区分了三类状态：

- app 使用记录 / ranking
- session 级审批
- instruction 是否已经向某 app 投递过

这比“全局开一个 computer-use 权限开关”要细很多。

### 11. 在真实 tool 调用期间，仍然没有看到 client-service 之间的常驻 Unix socket

在实际调用一次 `get_app_state(Finder)` 之后，重新查看进程与句柄：

- `SkyComputerUseService`
  - 仍然只稳定持有一条 Unix socket
  - 对端是 `Codex.app` 的 `codex-ipc/ipc-501.sock`
- 多个存活的 `SkyComputerUseClient mcp`
  - 可见句柄基本只有：
    - stdio pipe 或 TTY
    - `Analytics.db`
    - 少量系统控制句柄
  - 没有观察到稳定的 client-service Unix socket

同时 service 还能看到：

- `No active Computer Use IPC client processes; terminating service`

所以当前更稳妥的结论是：

- service 会跟踪 active IPC client
- 但这个 client-service transport 至少不是一个容易通过 `lsof -U` 看到的常驻 Unix socket

这进一步支持以下几种可能性之一：

- XPC / NSXPC
- 每次请求短连、很快关闭的本地通道
- 其他由系统服务包装、不直接体现在普通 socket 句柄上的 transport

## 当前推断

### 1. 官方内部大概率是“三层协议栈”，而不是单一 MCP server

更符合现有证据的分层是：

1. Codex appserver IPC
   - service 和 Codex 宿主的私有 JSON-RPC / thread event 集成
2. ComputerUse IPC
   - client 和 service 之间的本地受信请求协议
3. MCP
   - client 对外暴露给 Codex agent runtime 的标准工具面

这解释了为什么：

- 外部看上去只是一条 `SkyComputerUseClient mcp`
- 但实际运行里还会出现 parent constraints、sender auth、service idle timeout、thread-ended cleanup

### 2. `get_app_state` 只是外部 API 名称，内部核心对象更像 `Skyshot`

开源版如果要追求结构清晰，内部最好也拆成：

- app targeting
- screenshot capture
- AX tree extraction
- app state serialization

而不是一上来就把所有逻辑耦合进一个 `get_app_state()` 函数里。

### 3. 公开 tools 比内部能力更保守

当前内部已可见但外部未公开的能力至少包括：

- app start
- app activate / deactivate
- app usage tracking

这说明官方公开的 9 个 tools 是一个更保守的产品化裁剪，不等于 service 的全部能力。

## 当前未决问题

- `ComputerUseIPC*` 这层最终到底走 XPC、NSXPC、私有 socket，还是别的本地 transport？
- `AppStartRequest` 在什么场景会被真正使用，是否只用于首个 app 会话建立？
- `AppUsageRequest` 是纯 analytics / ranking，还是也参与审批和安全策略？
- `Skyshot` 的精确定义是什么，是否包含屏幕截图、窗口元数据、AX 树和 URL 等全部上下文？
- sender authorization 的 requirement 是否直接绑定到 OpenAI Team ID，还是还有更细的 bundle / code requirement 约束？
- 如果不是 XPC，client-service 的本地 transport 为什么在真实调用期间仍然几乎不可见？
