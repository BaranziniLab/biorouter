# Remaining Computer Use Tool Alignment

## 目标

在 `click` 和 `set_value` 已完成官方姿势收口后，逐项逆向并对齐剩余 7 个 Computer Use tools，重点确认它们是否会抢用户真实鼠标或抢前台焦点，并把结论、实现差距、测试和文档状态落到仓库里。

## 范围

- 包含：
  - `list_apps`
  - `get_app_state`
  - `perform_secondary_action`
  - `scroll`
  - `drag`
  - `type_text`
  - `press_key`
  - 官方 bundled `computer-use` 当前版本的 `tools/list`、静态字符串、导入符号、必要时的反汇编定位。
  - 每个工具的本地实现差距、行为修正、测试、架构文档和 history。
- 不包含：
  - 重新打开已经完成的 `click` / `set_value` 行为修正，除非剩余工具逆向时发现共用底层需要补丁。
  - 复刻官方闭源 host authorization、私有 IPC、完整 visual cursor choreography。
  - 用全局物理鼠标事件作为默认兜底来换取表面通过率。

## 背景

- 相关文档：
  - `docs/ARCHITECTURE.md`
  - `docs/references/codex-computer-use-reverse-engineering/tool-call-samples-2026-04-17.md`
  - `docs/references/codex-computer-use-reverse-engineering/internal-ipc-surface.md`
  - `docs/histories/2026-04/20260421-2120-disable-click-global-pointer-default.md`
  - `docs/histories/2026-04/20260422-1050-align-set-value-settable-boundary.md`
- 相关代码路径：
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ToolDefinitions.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseToolDispatcher.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/InputSimulation.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
  - `apps/OpenComputerUseFixture/Sources/OpenComputerUseFixture/main.swift`
  - `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- 已知约束：
  - 官方 `1.0.755` 的 `tools/list` 里 `scroll.pages` 是 `number`，文案为 `Number of pages to scroll. Fractional values are supported. Defaults to 1`；旧安装根可能仍暴露 `integer` 旧 schema。
  - 官方 binary 暴露 `MouseEventTarget`、`KeyboardEventTarget`、`EventTap`、`SyntheticAppFocusEnforcer`、`SystemFocusStealPreventer`、`UIElementScrollOperation`、`ScrollAreaUIElement`、`ScrollBarUIElement` 等类型名，说明动作路由不是简单把所有 fallback 发到全局 HID 光标。
  - 本地 `drag` / `scroll` 已完成默认路径修正：全局 `.cghidEventTap` 仅在 `OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS=1` 时启用，默认改走 AX 或 pid-targeted event；官方私有 `MouseEventTarget` / `UIElementScrollOperation` 只做静态证据确认，不在本轮复刻闭源内部实现。

## 风险

- 风险：只对齐 schema，不处理全局事件 fallback，用户仍可能遇到鼠标或焦点被抢。
  - 缓解方式：每个动作工具都明确写下默认执行路径和 fallback 是否允许；高风险物理指针路径必须显式 opt-in 或替换为更窄的目标路由。
- 风险：官方行为通过私有 AccessibilitySupport 类型实现，开源版只能近似。
  - 缓解方式：区分“确认一致”、“确认不同但有安全替代”和“待逆向”；不要把猜测写成结论。
- 风险：改动 `drag` / `scroll` 可能影响 smoke fixture。
  - 缓解方式：fixture bridge 保持 deterministic；真实 app 路径另补单元测试覆盖参数解析和 fallback gate。

## 工具级 TODO

- [x] `click`: 已对齐 element-targeted AX 优先、`click_count` 重复 AX action、默认禁止全局物理指针 fallback。
- [x] `set_value`: 已对齐 `AXUIElementIsAttributeSettable(kAXValueAttribute)` 前置检查，非 settable 返回官方风格错误，不退到键盘/剪贴板/未公开文本替换。
- [x] `list_apps`: 复核官方 `1.0.755` 输出字段、排序和 denylist 影响；确认本地 Spotlight + running app 合并仍一致。
- [x] `get_app_state`: 复核官方 session/start-state、截图、AX tree rendering、stale-state 错误和不抢前台策略；确认本地不 `activate` 的边界。
- [x] `perform_secondary_action`: 复核官方 action name 匹配、菜单项/secondary action 错误语义和是否需要 prepare interaction；确认本地 AX action 路径不会抢焦点。
- [x] `scroll`: 对齐官方 `pages` number schema 和 fractional pages；静态确认 `UIElementScrollOperation` / scroll bar 类型线索，消除默认全局物理事件 fallback。
- [x] `drag`: 静态确认 `MouseEventTarget` / drag dispatch 类型线索；消除默认全局 mouse event fallback，保留显式 opt-in。
- [x] `type_text`: 逆向 `KeyboardEventTarget` / keyboard layout 错误语义；确认本地 `postToPid` 不抢焦点，并对齐缺失 text / Unicode 边界。
- [x] `press_key`: 逆向 xdotool key parser、keyboard layout、modifier 语义和错误文案；确认本地 `postToPid` 不抢焦点。

## 里程碑

1. 确认剩余 7 个工具的官方 `1.0.755` schema、静态字符串、导入符号和当前本地差异。
2. 完成 `scroll` / `drag` 默认非物理指针路径、fractional scroll、required 参数错误和 secondary action 错误语义收敛。
3. 完成 `list_apps` / `get_app_state` / `type_text` / `press_key` 的复核结论、测试、文档、history 和最终状态清理。

## 验证方式

- 命令：
  - `swift test`
  - `swift build --product OpenComputerUse`
  - `COMPUTER_USE_PLUGIN_ROOT="$HOME/.codex/plugins/cache/openai-bundled/computer-use/1.0.755" go run . list-tools --transport app-server`
  - `go run . list-tools --transport direct --server-bin ../../.build/debug/OpenComputerUse`
  - `./scripts/run-tool-smoke-tests.sh`
- 手工检查：
  - `tools/list` 与官方 `1.0.755` schema 对齐，尤其 `scroll.pages`。
  - 每个动作工具都能说明是否使用 AX、pid-targeted event、window-targeted event 或显式 opt-in 物理 pointer fallback。
  - 不允许在默认路径里调用会移动系统硬件光标的 mouse move / drag 兜底。
- 观测检查：
  - 运行真实 app 样本时，用户硬件鼠标位置不应因为默认工具调用改变。
  - action 后返回仍包含最新 state text 和截图。

## 进度记录

- [x] 确认剩余 7 个工具的官方 `1.0.755` schema、静态字符串、导入符号和当前本地差异。
- [x] 完成 `scroll` / `drag` 默认非物理指针路径、fractional scroll、required 参数错误和 secondary action 错误语义收敛。
- [x] 完成 `list_apps` / `get_app_state` / `type_text` / `press_key` 的复核结论、测试、文档、history 和最终状态清理。

## 决策记录

- 2026-04-22：将剩余 7 个工具拆成独立 checklist；`drag` 和 `scroll` 因存在全局事件 fallback 排在最前，键盘类和只读类随后复核。
- 2026-04-22：官方 `1.0.755` 的 `scroll.pages` 已确认是 `number` schema；旧插件根返回的 `integer` 视为旧版本基线，不再作为当前对齐目标。
- 2026-04-22：官方 app-server 对 required string 的空字符串按 missing 处理；本地 dispatcher 统一改成非空 required string，并返回 `Missing required argument: <name>`。
- 2026-04-22：本地 `scroll` / `drag` 不再默认调用全局 `.cghidEventTap` 和 app activation fallback；未命中 AX scroll action 时先用 `CGEvent.postToPid` 定向发给目标进程，只有显式打开 `OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS=1` 才走物理指针兜底。
- 2026-04-22：`perform_secondary_action` 保持 AX action 路径，invalid action 错误改为官方字符串形态；fixture 的 `Raise` 不再调用 global pointer prepare。
- 2026-04-22：官方 binary key table 包含 `BackSpace`、`Page_Up`、`Prior`、`Next`、`F1...F12` 和完整 `KP_0...KP_9/KP_Enter` 等 xdotool 名称；本地 `press_key` parser 补齐这些常用 alias，仍通过 `CGEvent.postToPid` 定向投递。
- 2026-04-22：`list_apps` / `get_app_state` 本轮没有新增代码路径：当前实现已按官方 surface 输出运行中 + 近 14 天 app，并且 `get_app_state` 不主动 `activate` 目标 app；验证以官方/本地 `tools/list`、smoke suite 和既有 reverse-engineering 样本为准。
