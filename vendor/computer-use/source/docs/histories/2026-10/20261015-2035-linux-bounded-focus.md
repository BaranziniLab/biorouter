# Bound Linux focus lookup and reuse its result

Agent: Codex; runtime: Linux.

User request: submit scoped improvements for the remaining Linux performance paths.

Linux snapshots currently enumerate the app/window twice and recursively locate the focused element separately for its summary and selected text. This PR resolves focus once in the window already selected for the snapshot and applies the snapshot's node/depth budgets. If traversal ends before focus is found, the returned text explicitly reports an incomplete search rather than claiming focus is absent.

Full AX tree/index and screenshot behavior is unchanged. The focus summary and selection come from the same element. A very deep or large focus search may now return incomplete metadata; callers can explicitly raise the existing tree budgets. This is a cooperative traversal budget, not a way to interrupt a blocking AT-SPI call.

Validation: Python bridge tests (14 passed), including shared focus lookup, node/depth bounds, complete absence and explicit truncation reporting. No live desktop benchmark is claimed.

Changed files: `apps/OpenComputerUseLinux/runtime.py`, `apps/OpenComputerUseLinux/runtime_test.py`, `docs/linux-bounded-focus.md`, architecture and release notes.


## 2026-10-17：基于当前 main 复查

用户要求在最新 main 上解决冲突、检查测试并补充提交。合入 `1b6fd8f`（含 #100），保留两组 Python 测试和全部发布记录。

### 补充修复

- 原实现到达深度上限后仍逐个读取下一层的子节点，但这些节点不计入 visited，宽树下可绕过节点预算。现在在读取下一层前停止，同时区分边界叶节点的完整查找与有未读子节点的不完整查找。
- 原选区路径调用当前 AT-SPI 不存在的 `get_text_selections`，导致选中文字被静默省略。改用 `get_n_selections` / `get_selection` / `get_text`；摘要与选区仍来自同一个焦点元素，读取失败保留摘要。
- 增加宽树深度边界、叶节点、剪枝后找到焦点、Unicode 偏移与文本限制、无选区和选区读取失败的回归。
- 扩展 #100 的隔离 GTK fixture，新增 `--focus-checks`；更新架构、使用文档和功能记录。

### 验证

- 新回归在补修前有 4 项失败；修复后 Python 全套 24 项通过，包含 #100 的动作回读测试。
- Debian 12 ARM64 临时容器中，完整 Linux Go 测试二进制与 Node 24 的 36 项测试通过；Go 测试由宿主 Go 1.26.1 交叉编译，实际在 Linux 执行。
- native MCP / 真实 GTK 验证通过：节点和深度预算提示、提高预算后的中文 / emoji 选区、text_limit 截断、真实按键清除选区后的焦点摘要。原有动作回读 smoke 也通过。
- Linux-targeted go vet、格式检查、ARM64 / AMD64 构建、shell / Python 语法、文档与 whitespace 检查通过。
- 使用已有容器引擎及私有 D-Bus / Xvfb，未启动桌面虚拟机；不操作个人应用。验证限于 X11 / GTK / AT-SPI，不声称 Wayland 或整任务性能收益。
