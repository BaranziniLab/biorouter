# Linux 隐藏控件详情

默认读取完整详情。设置 `OPEN_COMPUTER_USE_LINUX_LIGHTWEIGHT_HIDDEN=1` 后，AT-SPI 明确标为非 `SHOWING` 的控件会省略位置、值、动作、toolkit 和 automation ID，行内显示 `[hidden details omitted]`。

```sh
OPEN_COMPUTER_USE_LINUX_LIGHTWEIGHT_HIDDEN=1 ocu js \
  'var app = await cua.getApp("gedit"); await app.getAXState();'
```

节点、索引、路径、角色和名称仍保留，也不会因为父节点隐藏就跳过子节点。状态读取失败或未知时继续读取完整详情；负坐标和窗口外位置不作为隐藏依据。控件重新显示后，下次观察会恢复详情。

开启后每个节点会额外读取一次 `SHOWING` 状态，适合隐藏节点较多的界面，不保证所有界面更快。它不改变焦点查找、截图或动作投递，也不会自动打开。

## 测试

```sh
python3 apps/OpenComputerUseLinux/runtime_test.py
(cd apps/OpenComputerUseLinux && go test ./...)
```

真实 GTK 测试复用 [私有 D-Bus / Xvfb 环境](linux-action-read-back.md)，不需要完整桌面或虚拟机：

```sh
(cd apps/OpenComputerUseLinux && go build -o /tmp/ocu-hidden-test .)
bash scripts/run-linux-read-back-smoke.sh /tmp/ocu-hidden-test --hidden-checks
```

脚本创建两页 GTK 标签页，对比开关前后的完整树、索引、路径和属性辅助函数调用次数，再通过 native MCP 切换页面、读取恢复的详情，并验证点击、输入和动作回读。属性计数调用的仍是真实 AT-SPI 接口；单测另覆盖状态异常和隐藏父节点下的可见子节点。

本次 X11 / GTK 3 / AT-SPI 实测为 12 个节点、3 个隐藏节点，四类属性辅助函数共调用 48 → 36 次。这不是 D-Bus 消息计数或耗时基准，不代表 Wayland、其他 toolkit 或完整任务的速度。
