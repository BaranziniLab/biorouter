# Linux 焦点查找

快照复用已经选定的窗口，只查找一次焦点，再从同一个元素读取焦点摘要和选中文字，省去重复的应用 / 窗口枚举与树遍历。

焦点查找单独遵守 `get_app_state` 的 `max_tree_nodes` / `max_tree_depth` 上限，默认 1200 个节点、64 层。到达深度上限后不再读取下一层子节点。预算不足且未找到焦点时，返回：

```text
[Focus search incomplete: node/depth budget reached.]
```

这表示查找未完成。需要时可以提高预算后重试。完整 AX tree、索引和截图仍走原来的路径；遍历预算也无法中断某一次阻塞的 AT-SPI 调用，外层 bridge 仍有超时保护。

选区使用标准 AT-SPI `get_n_selections` / `get_selection` / `get_text` 接口，按字符偏移读取，支持中文和 emoji，并遵守 `text_limit`。没有选区或选区读取失败时仍可返回焦点摘要。

## 测试

回归测试：

```sh
python3 apps/OpenComputerUseLinux/runtime_test.py
```

在 Linux 上构建后，可复用 [GTK smoke 环境](linux-action-read-back.md) 验证真实焦点和选区：

```sh
(cd apps/OpenComputerUseLinux && go build -o /tmp/ocu-focus-test .)
bash scripts/run-linux-read-back-smoke.sh /tmp/ocu-focus-test --focus-checks
```

脚本在私有 D-Bus / Xvfb 中创建自己的窗口，验证节点 / 深度预算提示、提高预算后的选区、中文 / emoji、文本截断，以及清除选区后的焦点摘要。覆盖 X11 / GTK / AT-SPI，不代表 Wayland 或其他应用的性能测试。
