## [2026-04-17 11:56] | Task: 沉淀 computer-use 逆向分析基础

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 分析 `SkyComputerUseClient` 和 `Codex Computer Use.app`，并把结果按仓库规则持续落到 `docs/` 的独立目录里，后续再根据收集结果考虑实现开源版。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Execution Plan]**: 新建长期执行计划，明确先调研、后实现的推进方式。
- **[Research Docs]**: 新建独立逆向分析目录，记录 baseline architecture 和 runtime / host dependency 结论。
- **[Docs Index]**: 更新 `docs/references/README.md`，把这组分析资料纳入仓库索引。
- **[Probe Script]**: 新增最小 `stdio` probe 脚本，用来直接复现 `SkyComputerUseClient mcp` 在 pipe/stdin 模式下的启动与退出行为。

### 🧠 Design Intent (Why)
这项工作跨多轮且涉及闭源 bundle 逆向分析，不能把核心结论留在聊天上下文里。先把长期计划和独立分析目录建起来，后面每轮都能在同一组文档里累积证据、修正判断，再收敛到开源实现方案。

### 📁 Files Modified
- `scripts/probe-cua-stdio.js`
- `docs/exec-plans/active/20260417-open-source-computer-use-reverse-engineering.md`
- `docs/references/README.md`
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:10] | Task: 用 Python MCP SDK 直接复现 computer-use 直连失败

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 不用 Inspector，直接 init 一下 uv，然后用 Python MCP SDK 自己写个最小 client 去试。

### 🛠 Changes Overview
**Scope:** `pyproject.toml`, `uv.lock`, `scripts/`, `docs/`

**Key Actions:**
- **[UV Bootstrap]**: 初始化仓库级 `uv` Python 项目骨架，并安装 `mcp` 依赖。
- **[Python Probe]**: 新增 `scripts/probe_cua_mcp_python.py`，使用 Python `mcp` SDK 的 `stdio_client + ClientSession.initialize()` 做最小直连实验。
- **[Runtime Evidence]**: 通过 Python SDK 复现 `initialize` 阶段的 `BrokenResourceError`，并确认系统侧同时生成新的 `Launch Constraint Violation` crash report。
- **[Docs Sync]**: 将 Python SDK 复现路径和结论补充进逆向分析文档与 execution plan。

### 🧠 Design Intent (Why)
Inspector 包了一层代理和前端交互，失败面太大。用官方 Python MCP SDK 直接复现之后，可以把问题压缩到最小：`stdio` 进程拉起成功，但一到 `initialize` 写入阶段就断管，同时系统记录 launch constraint kill。这个证据链比 Inspector 更干净，适合继续逆向。

### 📁 Files Modified
- `pyproject.toml`
- `uv.lock`
- `scripts/probe_cua_mcp_python.py`
- `docs/exec-plans/active/20260417-open-source-computer-use-reverse-engineering.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:18] | Task: 收敛 Codex 宿主 IPC 与 parent constraint 证据

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Parent Constraint]**: 通过存活进程父链和签名信息确认，能长期存活的 `SkyComputerUseClient mcp` 基本都挂在 OpenAI Team ID `2DC432GLL2` 的 Codex 宿主进程下面。
- **[Appserver IPC]**: 通过 `lsof` 和 strings 确认 `SkyComputerUseService` 会连接 `Codex.app` 持有的 `codex-ipc/ipc-501.sock` Unix socket。
- **[Host Integration]**: 补充 service 中关于 `CodexAppServerJSONRPCConnection`、auth cache/provider、plugin 安装和 notify hook 更新的实现痕迹。

### 🧠 Design Intent (Why)
到这一步，问题已经不只是“外部 stdio 拉不起官方 client”，而是“官方 computer-use 依赖一个私有的 Codex 宿主 IPC 层”。把这层关系写清楚之后，开源版可以更明确地选择：直接去掉私有宿主耦合，而不是误把它当成 computer-use 内核的必要组成。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:04] | Task: 补充 computer-use tools schema

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 基于现有的分析，继续分析 codex computer use 的所有 tools 的 name 和参数；并把工具定义的 schema 用 code block 结构化整理进文档，方便查看。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Tool Schemas]**: 在 baseline architecture 文档中补全 `computer-use` 当前公开的 9 个 tools，并为每个 tool 增加结构化 code block schema。
- **[Runtime Notes]**: 基于当前会话实际调用 `list_apps` 和 `get_app_state` 的结果，补充返回形态和使用语义。
- **[Compatibility Notes]**: 增加面向开源兼容层的接口层判断，说明 `get_app_state`、`element_index` 和坐标模式的角色边界。

### 🧠 Design Intent (Why)
前一轮文档只确认了 tool 名单，没有把参数面和调用语义整理成能直接拿来设计兼容层的格式。把 schema 结构化落到仓库里之后，后续做开源 MCP 复刻时可以直接从这份文档抽接口，而不需要再回聊天记录里找零散结论。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:34] | Task: 补充打包结构与 turn-ended 生命周期集成

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Packaging Layout]**: 新增独立文档，记录官方插件根目录、`.codex-plugin/plugin.json`、`.mcp.json` 和主 app 的分发结构。
- **[Lifecycle Hook]**: 结合 `SkyComputerUseClient --help`、`~/.codex/config.toml` 和 `Codex` 宿主 strings，确认 `turn-ended` 是挂在 legacy notify / `after_agent` 生命周期上的回调入口。
- **[Distribution Evidence]**: 补充 `LSUIElement`、Sparkle 更新源、内嵌 resource bundles、application group container 和 provisioning profile 的观察结果。

### 🧠 Design Intent (Why)
前面的文档已经把 runtime 宿主依赖写清了，但“这个插件是怎么被 Codex 看见的”和“`turn-ended` 到底属于哪条生命周期链”还没有独立沉淀。把这层接口面单独整理出来后，后续设计开源版时就能更清楚地区分哪些是 MCP 能力本体，哪些只是官方宿主集成方式。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/packaging-and-lifecycle-integration.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:47] | Task: 收敛 client-service 内部 IPC 面

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Internal IPC]**: 新增独立文档，整理 `ComputerUseIPC*` 类型族，确认 client 和 service 之间还存在一层独立于 MCP 的本地协议。
- **[Request Families]**: 收敛 `ListAppsRequest`、`AppGetSkyshotRequest`、`AppPerformActionRequest`、`AppModifyRequest`、`CodexTurnEndedRequest`、`AppUsageRequest` 等 request 族。
- **[Security Boundary]**: 补充 `ComputerUseIPCSenderAuthorization`、`CodeSignature`、`Requirement` 等 sender auth 证据，并记录 service 的 active-client / idle-timeout 自回收语义。

### 🧠 Design Intent (Why)
到这一步，官方实现已经很明显不是“一个 MCP server 直接碰系统 API”，而是至少有一层内部 IPC 把 MCP client 和桌面 automation service 隔开。把这层协议栈单独落文档后，后续开源版就能更明确地决定是否保留这层分离，以及如果保留，应如何把 sender auth 和 skyshot 模型设计成透明、可实现的开源方案。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/internal-ipc-surface.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:56] | Task: 用真实 tool 调用补强内部 IPC transport 判断

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Live Runtime Check]**: 通过一次真实 `computer-use` 的 `get_app_state(Finder)` 调用拉起 service，再用 `lsof`/`pgrep` 复查进程和句柄。
- **[Transport Evidence]**: 确认 service 侧持久可见的 Unix socket 仍然只有连接 Codex 宿主的 `codex-ipc/ipc-501.sock`，没有观察到 client-service 常驻 Unix socket。
- **[Inference Tightening]**: 把 client-service transport 的判断进一步收紧到“更像 XPC、短连本地通道，或其他不可见 transport”，避免误写成普通本地 socket。

### 🧠 Design Intent (Why)
前一轮虽然已经从类型名里确认了 `ComputerUseIPC*` 协议族，但 transport 形式还只是猜测。补一条真实调用期的运行时反证后，可以更明确地排除“client 和 service 之间长期维持一条普通 Unix socket”这种过度简化的实现想象。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/internal-ipc-surface.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 13:03] | Task: 收敛 turn-ended payload wire shape 与 caller 限制

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Wire Shape]**: 通过官方 `openai/codex` 开源仓库里的 `codex-rs/hooks/src/legacy_notify.rs`，确认 `notify` / `after_agent` 历史兼容 payload 的精确 JSON 结构。
- **[Lifecycle Mapping]**: 将该 JSON wire shape 映射回 `SkyComputerUseClient turn-ended <payload>`，把原先“很可能”升级成带源码依据的结论。
- **[Action-Time Validation]**: 本机直接从外部 shell 执行 `turn-ended` 的合法和非法 payload，均观察到 `status=137` 与 `Launch Constraint Violation` crash report，确认它也不属于任意 caller 可调用的独立 CLI。

### 🧠 Design Intent (Why)
之前我们已经知道 `turn-ended` 属于 lifecycle 集成，但还差两块硬证据：一是 payload 到底长什么样，二是它是否能被外部 caller 当作通用命令直接使用。把这两点都钉住之后，后续开源版就能更明确地把“生命周期通知的结构”与“官方受信宿主限制”拆开处理。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/packaging-and-lifecycle-integration.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 13:10] | Task: 收紧 previous-notify 的边界判断

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Core Comparison]**: 对照官方 `openai/codex` 开源核心的 `config/mod.rs`、`hooks/legacy_notify.rs`、`hooks/registry.rs`，确认核心 `notify` 机制本身只认 argv + 尾部 JSON payload，没有 `previous-notify` 概念。
- **[Boundary Tightening]**: 将 `--previous-notify` 的判断从“可能是链式参数”收紧为“更可能是 `SkyComputerUseClient turn-ended` 自己引入的兼容包装参数，而非 Codex 核心 hook API”。
- **[Open Question Update]**: 把未决问题聚焦到“这个参数的具体编码格式”而不是“它是否属于核心 notify 机制”。

### 🧠 Design Intent (Why)
这一步的价值在于把“官方宿主自己的兼容包装”与“Codex 核心公开 hook 机制”拆开。后续做开源版时，就不会误以为 `previous-notify` 是必须兼容的 Codex 核心协议字段，而会把它当成官方插件接管 notify 配置时的一个私有迁移细节。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/packaging-and-lifecycle-integration.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:41] | Task: 收敛权限 onboarding 与拖拽引导窗口线索

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 我发现在授权的时候会做一个窗口，让用户直接可以拖动 codex computer use.app 进去，而不需要自己去找。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Permission Evidence]**: 收敛 `Accessibility`、`Screen Recording`、`TCCDialogSystemPermission`、`permissionsWindow`、`permissionState` 等 strings，确认权限 gating 和权限窗口都属于主 service 的内建能力。
- **[Accessory UI Evidence]**: 补充 `SystemSettingsAccessoryWindow`、`SystemSettingsAccessoryTransitionOverlayWindow`、`ArrowWindow`、`SystemSettingsAccessoryWindowDragDelegate` 等命名，确认官方围绕 System Settings 做了自定义辅助 UI。
- **[Drag Flow Inference]**: 记录 `DraggableApplicationView`、`dragDelegate`、`dragContinuation`、`draggable` 等命名，并将其与用户观察到的“把 app 拖进去授权”体验对应起来。
- **[Docs Index]**: 新增独立权限 onboarding 文档，并更新逆向资料目录索引。

### 🧠 Design Intent (Why)
这条发现把官方体验从“只是检测 TCC 权限”推进到了“围绕 macOS 授权流程做了一层产品化引导”。对开源实现来说，这个差异很重要，因为它决定了首轮权限配置是否顺畅，也决定了我们后续是否要复刻一层 System Settings accessory window，而不只是弹一句提示文案。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/permission-onboarding.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:46] | Task: 提取 computer-use 的视觉资源样本

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 可以，你怎么扣视觉资源？你试试看？

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Asset Extraction]**: 使用 AppKit `Bundle.image(forResource:)` 直接从官方 bundle 中导出 `SoftwareCursor`、`HintArrow`、`CUAAppIcon_Assets/cursor`、`CUAAppIcon_Assets/cursor dark` 和 `menubar-cursor` 等资源为 PNG。
- **[Verification]**: 记录导出文件的尺寸、透明通道和哈希；确认 `Package_SlimCore.bundle` 与 `Package_ComputerUse.bundle` 中的 `SoftwareCursor` 导出后二进制一致。
- **[Asset Archive]**: 将导出的 PNG 归档到 `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/`，并新增资产索引说明。
- **[Doc Linking]**: 在软件光标和权限 onboarding 两份文档中补上对提取资源的直接链接，方便后续查看与比对。

### 🧠 Design Intent (Why)
前面的分析虽然已经能从 strings 和窗口行为推断软件光标、权限箭头这些 UI 存在，但直接把资源抠出来后，结论就从“命名层面可信”升级成了“已有可点击、可比对的实际视觉资产”。这对后续复刻开源版体验非常有帮助，因为可以直接参考尺寸、复用关系和大致视觉方向。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/assets/README.md`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/appicon-cursor-dark.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/appicon-cursor.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/hint-arrow.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/menubar-cursor.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/software-cursor-computeruse.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-17/software-cursor-slimcore.png`
- `docs/references/codex-computer-use-reverse-engineering/permission-onboarding.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

### Follow-up
- 将 `software-cursor-overlay.md` 和 `permission-onboarding.md` 里指向导出 PNG 的绝对本机路径链接改成仓库内相对路径，避免文档依赖作者机器目录结构。

## [2026-04-17 12:32] | Task: 分析黄色虚拟鼠标 overlay 的实现线索

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 现在工具调用足够了。然后我发现实际在使用中，codex computer use 会显化出一个黄色的小鼠标样式（应该就是一个浮动的图片，体验超级好，这样可以不抢用户的鼠标，但是同时又模拟出一个鼠标的动画，这个你能探测分析到么？）

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Resource Evidence]**: 补查 `Codex Computer Use.app` 的 `Assets.car` 和 `strings`，确认主 app 里存在 `cursor`、`cursor dark`、`menubar-cursor` 等资源命名，以及 `cursorWindow`、`imageView`、`Start Bezier cursor animation` 等实现线索。
- **[Runtime Probe]**: 用 `CGWindowListCopyWindowInfo` 直接枚举运行时窗口，确认 `Codex Computer Use` 进程名下存在一个名字就是 `Software Cursor` 的独立窗口。
- **[Behavior Check]**: 在安全的 Finder 点击前后比对 `Software Cursor` 的窗口坐标，确认它会随工具动作移动，而不是静态资源或菜单栏装饰。
- **[Docs Index]**: 新增独立文档专门收敛黄色虚拟鼠标 overlay 的事实和推断，并更新逆向资料目录索引。

### 🧠 Design Intent (Why)
这个黄色小鼠标是官方体验里非常关键的一层产品细节。把它从“感觉上像一个浮动图片”收敛成“`SkyComputerUseService` 持有的独立 `Software Cursor` 窗口”之后，后续开源实现就能更明确地把输入注入和视觉反馈拆成两套机制，而不是误以为必须直接操纵系统真实光标。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:21] | Task: 补充 computer-use tool call 实测样本

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 你有尝试调用所有的工具了么？最好是都能调用几次（比如针对不同的 app 或动作），比如 3 次，然后把调用的请求和响应结果都保存到 docs 里某个目录下，用作未来的参考。

### 🛠 Changes Overview
**Scope:** `docs/`

**Key Actions:**
- **[Live Samples]**: 对 `computer-use` 的 9 个公开 tools 做了实测，尽量为每个 tool 保留至少 3 个样本。
- **[Request/Response Archive]**: 新增独立 samples 文档，按 tool 归档 request / response code block，并注明截图省略和响应节选策略。
- **[Boundary Notes]**: 额外记录了 `get_app_state` 的 `appNotFound` 和 safety denial 两类边界响应，以及 `drag` 的 no-op 倾向样本。
- **[Docs Index]**: 更新逆向分析目录索引，把这份实测文档纳入当前文档列表。

### 🧠 Design Intent (Why)
只知道 tool schema 还不够，真正做兼容层时还要知道运行时响应是“纯状态快照”还是“只返回 OK”，以及不同工具的稳定性差异。把真实 request / response 样本沉淀进仓库后，后续实现和回归验证都可以直接拿这份文档当基线，而不需要重复手工试探。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/tool-call-samples-2026-04-17.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`

## [2026-04-17 12:51] | Task: 清理一次性 probe 脚本和临时 Python/uv 配套

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 帮我清理一下这个一次性脚本和配套的东西吧，好像还为此初始化了uv，uv也不要了，这不是python的repo

### 🛠 Changes Overview
**Scope:** `scripts/`, `docs/`

**Key Actions:**
- **[Probe Cleanup]**: 删除一次性的 Node / Python `stdio` probe 脚本，不再把它们作为仓库长期资产保留。
- **[Python Chain Removal]**: 移除为该实验临时引入的 `uv` 项目骨架和 Python 依赖文件。
- **[Docs Reframe]**: 把逆向文档和执行计划里的相关表述改成“历史实验结论”，不再暗示仓库当前仍维护 Python / `uv` 入口。
- **[History Sync]**: 在同一份 history 中补记这次清理，保留研究轨迹但收掉一次性实现残留。

### 🧠 Design Intent (Why)
这些脚本和 `uv` 初始化只服务于一次性的最小复现实验，不属于仓库长期边界。把证据留在文档里、把临时运行链路从仓库里拿掉，可以避免后续读者误把这里当成一个需要维护 Python 环境的多语言仓库。

### 📁 Files Modified
- `scripts/probe-cua-stdio.js`
- `scripts/probe_cua_mcp_python.py`
- `pyproject.toml`
- `uv.lock`
- `docs/exec-plans/active/20260417-open-source-computer-use-reverse-engineering.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `docs/histories/2026-04/20260417-1156-open-source-computer-use-research-foundation.md`
