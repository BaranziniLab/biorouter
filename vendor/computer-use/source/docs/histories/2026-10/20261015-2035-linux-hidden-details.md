# Add opt-in lightweight details for hidden Linux nodes

Agent: Codex; runtime: Linux.

User request: submit scoped improvements for the remaining Linux performance paths.

With `OPEN_COMPUTER_USE_LINUX_LIGHTWEIGHT_HIDDEN=1`, snapshot rendering retains every node, index, runtime path, role and name, but skips geometry/value/action/toolkit/automation-ID reads for nodes explicitly reported as non-SHOWING by AT-SPI. Such rows are marked `[hidden details omitted]`.

This option is disabled by default. Unknown or failing state queries keep full details. The renderer never prunes a hidden parent's descendants: visible menu children retain their full properties and original numbering. Geometry alone is not evidence that a node is hidden, so off-window/negative coordinates are not used as a filtering rule.

Validation: Python bridge tests (13 passed), including skipped property calls, unchanged default output, unknown state fallback, visible children of hidden parents and index/path preservation. No live desktop benchmark or claim that all offscreen nodes are optimized.

Changed files: `apps/OpenComputerUseLinux/runtime.py`, `apps/OpenComputerUseLinux/runtime_test.py`, `docs/linux-hidden-details.md`, architecture and release notes.

## 2026-10-17：同步主线并补真实 GTK 验证

用户要求基于最新 main 解决冲突、检查测试并推送原 PR，补充真实 GTK 覆盖；本机没有完整 Linux 桌面，不启动桌面虚拟机。

- 合入主线 `eb501f4`，保留贡献者原提交，同时保留主线动作回读、焦点与选区测试以及发布记录。
- 检查后未发现需要修改功能实现的问题。增加状态集获取失败、缺失、contains 异常以及可见节点负坐标的回归，确保异常不会误省略详情。
- 扩展已有 GTK smoke 的 `--hidden-checks`：创建两页 notebook，直接读取真实远程 AT-SPI 对象，对比所有节点的索引 / 路径 / 名称 / 角色，验证隐藏节点省略、显示节点记录不变，并包装真实属性辅助函数计数。通过 native MCP 验证输出、切换页面后详情恢复及原有动作链，fixture 独立导出状态确认动作结果。
- 真实测试使用现有容器引擎上的临时 Debian 12 ARM64 容器、私有 Xvfb / D-Bus、GTK 3 / AT-SPI 2。不操作宿主个人应用，不启动桌面 VM。

验证：Python 29 项通过；Linux Go 测试 25 项通过；ARM64 / AMD64 构建和 Linux-targeted vet 通过；隐藏控件 native MCP smoke、默认动作回读 smoke，以及开关两种状态下的焦点 / 中文 emoji 选区回归均通过。真实树有 12 个节点、3 个隐藏节点，四类属性辅助函数调用 48 → 36 次；这不是 D-Bus 消息数或耗时基准。文档与 whitespace 检查通过。

范围：X11 / GTK / AT-SPI；没有做 Wayland、其他 toolkit 或全任务性能测试。每节点会增加 SHOWING 查询，因此保持默认关闭，不承诺所有界面都提速。无版本 bump。

验证完成后已删除临时容器及独立构建制品，未影响已有容器服务。
