## [2026-10-15 20:23] | Task: Add opt-in selective Linux observation

### Execution Context
- Agent: Codex
- Runtime: Linux development environment

### User Request
Offer cheaper explicit observation as a compatibility-preserving option, following the existing lightweight app binding approach.

### Changes
- Add optional Linux `get_app_state` booleans `include_ax` and `include_screenshot`, both defaulting to true.
- Skip AX tree, focus and selection collection for screenshot-only reads; skip capture/PNG encoding for AX-only reads.
- Forward optional REPL `includeAX` and `includeScreenshot` without changing legacy calls.
- Explicitly invalidate prior AX indexes and their aliases after screenshot-only reads, retain current window geometry for coordinate actions, and report the invalidation.
- Add Python collection-path tests, Go protocol/cache/schema tests, Node compatibility tests, usage documentation and release notes.

### Design Intent
Changing `getScreenshot()` defaults would also change its implicit index refresh. Keep existing behavior and make selective observation opt-in. A screenshot cannot validate old indexes, including after a window/process switch, so clear them rather than guessing from matching titles. A fresh AX observation restores indexes.

### Validation
- Linux Go suite and Python bridge unit tests.
- Node REPL regression tests, including unchanged legacy request arguments.
- REPL tests: 23 passed. Full Node suite: 36/37 passed; the staged npm packaging test requires macOS `lipo` and fails identically on the unmodified base checkout on Linux.
- Documentation and whitespace checks.
- No live desktop timing, macOS runtime or Windows runtime validation is claimed. This change is independent of the action read-back fix and does not change input delivery or bridge process lifetime.

### Files Modified
- `apps/OpenComputerUseLinux/main.go`
- `apps/OpenComputerUseLinux/main_test.go`
- `apps/OpenComputerUseLinux/runtime.py`
- `apps/OpenComputerUseLinux/runtime_test.py`
- `scripts/node-repl/open-computer-use-repl.mjs`
- `scripts/node-repl/open-computer-use-repl.test.mjs`
- `skills/open-computer-use/references/usage.md`
- `docs/ARCHITECTURE.md`
- `docs/releases/feature-release-notes.md`

## 2026-10-17：同步主线、修复别名遗漏并补实测

用户要求处理 #101，基于最新 main 解决冲突、检查测试并推送，补真实 GTK；同时评估截图可选与未变化图片重复返回的关系。

- 合入主线 `7cb8bb7`（含 #103），保留原贡献提交及两边的 Go / Python 测试、架构和发布记录。将条件 AX 采集接入现有一次有界 focus_details，保留焦点预算提示及隐藏节点选项。
- 发现只取截图时，应用 PID 改变后较早观察的查询别名可能残留。新增独立测试先复现失败，再清理新旧 PID 对应的全部观察别名，同时保留其他应用缓存。
- JS 在两种观察通道同时关闭时提前拒绝，不产生 native 请求；Persistent / Worker 两种会话均测试参数、错误及旧调用兼容性。
- 扩展 `--observation-checks`：在真实 GTK 上包装生产采集函数，确认 AX-only 没有截图、image-only 没有树或焦点读取，完整模式同时采集；检查真实 PNG、中文 / emoji 选区、名称 / PID 别名下旧索引拒绝、坐标点击及 AX 刷新后的设值。独立 GTK 状态确认结果。
- 新增 Worker JS → native MCP → GTK smoke，验证选择通道、图片输出、旧索引异常、AX 恢复及默认完整观察。环境为现有引擎上的临时 Debian 12 ARM64 容器、私有 Xvfb / D-Bus、GTK 3 / AT-SPI 2 和 Node 24，不启动桌面 VM。

验证：Python 35 项、Linux Go 30 项、Node 44 项通过；Linux ARM64 / AMD64 构建、targeted vet、shell / Python / Node 语法检查通过；按需观察真实链路、原有默认动作回读、焦点 / Unicode 选区和隐藏详情 smoke 均通过。文档与 whitespace 检查通过。未运行 Wayland 或其他 toolkit，不宣称全任务提速，无版本 bump。

需求评估：#101 提供 Linux 显式通道选择，不做截图去重，也未实现其他平台的通道开关。现有 macOS / Linux JS 动作已禁用自动回读，getAXState 不输出图片，但仍可能浪费 native 采集。未变化图片省略通常仍需截图后比较，主要节省传输和模型输入；后续的跨平台支持、会话 / 应用 / 窗口隔离、首帧 / 重置 / 强制返回和精确图像比较已记录到技术债追踪，不以 AX 未变作为画面未变的证据。

验证结束后已删除临时容器和独立构建制品，未影响其他容器服务。

## 2026-10-17：按用户要求统一三个平台

用户明确要求扩展 macOS / Windows，而不只实现 Linux。本阶段取代前文的 Linux-only 范围；自动图片去重仍单独跟踪。

- macOS 新增观察选项、严格布尔校验、schema / dispatcher / app-agent CLI 转发，截图关闭时跳过捕获和编码，AX 关闭时跳过树、选区、菜单及懒加载重试。fixture 同步行为。纯截图清理旧进程快照别名和 query 索引，旧别名不能触发隐式刷新继续使用旧编号。
- Windows Go / PowerShell 支持同样的参数、默认和空观察拒绝；按通道跳过生产采集函数，防御性清理结果中的禁用通道及旧缓存。新增纯传输测试接口，不改变生产 Windows / PowerShell 宿主限制。
- 三个平台的 snapshot CLI 新增 --include-ax / --include-screenshot true|false。只取 AX 后拒绝截图坐标输入，保留元素操作。macOS CLI 改为打印所有文本说明但不输出图片字节，修复实测发现的省略原因丢失。
- macOS 正常签名 release + 自有 AppKit 窗口实测通过：CLI / MCP / Worker JS、PNG、timing 分支证据、普通 / query 索引失效、坐标拒绝与 AX 恢复；窗口不能成为 key/main，未修改系统准入或权限。
- 本地检查：Swift 382 项（8 跳过、0 失败），观察专项 8 项；Linux Go 31 项、Python 35 项、Node 44 项；Windows Go 29 项与生产 PowerShell 函数分支测试通过；Linux / Windows 双架构构建及 targeted vet 通过。真实 GTK 观察、焦点、隐藏详情及 macOS 原生工具 / cursor smoke 通过。
- 增加专门的 Windows Actions 检查与临时 WPF 夹具，真实 Windows 2025 桌面上的 Go / Windows PowerShell 5.1 / native MCP / CLI / Worker JS 全部通过。覆盖三个通道模式、旧索引拒绝、AX 恢复、真实 Unicode 设值和坐标拒绝；正常窗口返回 PNG，物理 1×1 Win32 小窗口、最小化及 DWM-cloaked 窗口不返回图片，恢复后重新返回 PNG。工作流使用仓库既有固定 SHA actions 和 contents:read，无发布步骤。
- 初版 WinForms 控件不提供可写 ValuePattern，正向设值测试改用标准 WPF TextBox，没有扩大生产输入 fallback。夹具状态文件用无 BOM UTF-8 原子写入，避免 Windows PowerShell 空字符串与 null 的文件替换差异。精确 1×1 用同进程 Win32 窗口，避开 WPF 高 DPI 最小尺寸取整。
- Windows [完整实测记录](https://github.com/iFurySt/open-computer-use/actions/runs/39018965035) 通过于 `ae8ec2f`。覆盖 AppKit / GTK / WPF，不宣称所有 toolkit、Wayland 或完整任务性能均已验证。
- 文档同步为三平台使用方式、可复现测试命令、Windows CI 和当前覆盖范围；原 execution plan 完成归档，PR 保持开放，未执行合并。
