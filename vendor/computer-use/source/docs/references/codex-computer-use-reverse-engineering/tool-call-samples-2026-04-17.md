# Computer Use Tool Call Samples (2026-04-17)

这份文档记录 2026-04-17 在当前 Codex 会话里，对 `computer-use` MCP 的 9 个公开 tools 做的实测样本。

## 记录方式

- 样本来源：当前会话内的真实 MCP 调用结果。
- 目标：给后续开源兼容层实现保留“请求长什么样、响应大概长什么样”的仓库内参考。
- 文本策略：尽量保留原始响应格式；对特别长的 accessibility tree 只保留前缀和关键变化片段。
- 截图策略：`get_app_state`、`click`、`scroll`、`drag` 等调用都会在工具 UI 中附带截图；文档里只保留文本响应，不嵌入截图。
- 测试 app：`Finder`、`Activity Monitor`、`System Settings`。
- 安全边界：只选择了目录选择、搜索框输入、滚动、分隔条拖拽这类低风险动作，没有切换系统权限开关，也没有双击打开文件。
- 2026-04-17 晚些时候再次核对后确认：直接看 MCP tool `content[0].text` 时，官方文本是从 `App=...` 起头的；这里早先保留下来的 `Computer Use state (CUA App Version: 750)` / `<app_state>` 包裹不应再视为当前官方基线。

## `list_apps`

### Sample 1

Request

```json
{}
```

Response excerpt

```text
[{"type":"text","text":"Google Chrome — com.google.Chrome [running, last-used=2026-04-17, uses=20980]
PyCharm — com.jetbrains.pycharm [running, last-used=2026-04-17, uses=3444]
iTerm2 — com.googlecode.iterm2 [running, last-used=2026-04-17, uses=3178]
Sublime Text — com.sublimetext.4 [running, last-used=2026-04-17, uses=2023]
ChatGPT — com.openai.chat [running, last-used=2026-04-17, uses=661]
Finder — com.apple.finder [running, last-used=2026-04-17, uses=366]
...
Obsidian — md.obsidian [last-used=2026-04-08, uses=8]"}]
```

### Sample 2

Request

```json
{}
```

Response excerpt

```text
[{"type":"text","text":"Google Chrome — com.google.Chrome [running, last-used=2026-04-17, uses=20983]
PyCharm — com.jetbrains.pycharm [running, last-used=2026-04-17, uses=3447]
iTerm2 — com.googlecode.iterm2 [running, last-used=2026-04-17, uses=3179]
...
Obsidian — md.obsidian [last-used=2026-04-08, uses=8]"}]
```

### Sample 3

Request

```json
{}
```

Response excerpt

```text
[{"type":"text","text":"Google Chrome — com.google.Chrome [running, last-used=2026-04-17, uses=20983]
PyCharm — com.jetbrains.pycharm [running, last-used=2026-04-17, uses=3447]
iTerm2 — com.googlecode.iterm2 [running, last-used=2026-04-17, uses=3179]
...
Obsidian — md.obsidian [last-used=2026-04-08, uses=8]"}]
```

已观察到的返回形态：

```text
- 外层是 content array。
- 当前只返回一个 text block。
- text block 内部是多行纯文本，每行格式接近：
  App Name — bundle.id [running, last-used=YYYY-MM-DD, uses=N]
```

## `get_app_state`

### Sample 1

Request

```json
{
  "app": "Finder"
}
```

Response excerpt

```text
App=com.apple.finder (pid 1106)
Window: "open-codex-computer-use", App: Finder.
    0 standard window open-codex-computer-use, ID: FinderWindow, Secondary Actions: Raise
        1 split group
            2 scroll area
                3 outline sidebar
                    4 row (selectable, expanded) Value: Favorites, Secondary Actions: Collapse
                    ...
            25 scroll area
                26 outline Description: list view, ID: ListView
                    68 row (selectable, collapsed) Secondary Actions: Expand
                    119 row (selectable, collapsed) Secondary Actions: Expand
                    130 row (selected)
...
```

### Sample 2

Request

```json
{
  "app": "Activity Monitor"
}
```

Response excerpt

```text
App=com.apple.ActivityMonitor (pid 988)
Window: "Activity Monitor", App: Activity Monitor.
    0 standard window Activity Monitor – All Processes, Secondary Actions: Raise
        1 scroll area Secondary Actions: Scroll Left, Scroll Right, Scroll Up, Scroll Down
            2 outline Processes
                3 row (selectable) OrbStack Helper
                4 row (selectable) iTerm2
                ...
        36 toolbar
            41 Description: Categories, Help: Display processes in the category specified
                42 radio button Description: CPU, Value: 1
                43 radio button Description: Memory, Value: 0
            47 search text field (settable, string)
                48 button search
...
The focused UI element is 2 outline.
```

### Sample 3

Request

```json
{
  "app": "System Settings"
}
```

Response excerpt

```text
App=com.apple.systempreferences (pid 73431)
Window: "Screen & System Audio Recording", App: System Settings.
    0 standard window Screen & System Audio Recording, ID: main, Secondary Actions: Raise
        1 split group main, SidebarNavigationView
            2 container
                3 search text field (settable, string)
                5 scroll area Secondary Actions: Scroll Up, Scroll Down
                    6 list Sidebar
                        34 row (selected) Privacy & Security
            53 scroll area Secondary Actions: Cancel
                54 heading Screen & System Audio Recording
                ...
                    63 row (selectable) Codex Computer Use
                        64 switch Value: on, ID: Codex Computer Use_Toggle
...
```

额外边界样本：

Request

```json
{
  "app": "iTerm2"
}
```

Response

```text
[{"type":"text","text":"appNotFound(\"iTerm2\")"}]
```

Request

```json
{
  "app": "com.googlecode.iterm2"
}
```

Response

```text
[{"type":"text","text":"Computer Use is not allowed to use the app 'com.googlecode.iterm2' for safety reasons."}]
```

## `click`

### Sample 1

Request

```json
{
  "app": "Finder",
  "element_index": "68"
}
```

Response excerpt

```text
68 row (selected, collapsed) Secondary Actions: Expand
    69 cell (selected)
        71 text field (selected, settable, string) ... Value: docs
...
164 list path
    165 text ... Value: Macintosh HD Users leo projects github open-codex-computer-use docs
```

### Sample 2

Request

```json
{
  "app": "Finder",
  "element_index": "119"
}
```

Response excerpt

```text
119 row (selected, collapsed) Secondary Actions: Expand
    120 cell (selected)
        122 text field (selected, settable, string) ... Value: scripts
...
164 list path
    165 text ... Value: Macintosh HD Users leo projects github open-codex-computer-use scripts
```

### Sample 3

Request

```json
{
  "app": "Activity Monitor",
  "element_index": "43"
}
```

Response excerpt

```text
34 Description: Categories, Help: Display processes in the category specified
    35 radio button Description: CPU, Value: 0
    36 radio button Description: Memory, Value: 1
    37 radio button Description: Energy, Value: 0
...
The focused UI element is 2 outline.
```

## `perform_secondary_action`

### Sample 1

Request

```json
{
  "app": "Finder",
  "element_index": "68",
  "action": "Expand"
}
```

Response excerpt

```text
68 row (selectable, expanded) Secondary Actions: Collapse
    69 cell
        71 text field ... Value: docs
        72 disclosure triangle 1
79 row (selectable)
80 row (selectable)
81 row (selectable, collapsed) Secondary Actions: Expand
...
```

### Sample 2

Request

```json
{
  "app": "Finder",
  "element_index": "68",
  "action": "Collapse"
}
```

Response excerpt

```text
68 row (selectable, collapsed) Secondary Actions: Expand
    69 cell
        71 text field ... Value: docs
        72 disclosure triangle 0
```

### Sample 3

Request

```json
{
  "app": "Activity Monitor",
  "element_index": "0",
  "action": "Raise"
}
```

Response excerpt

```text
0 standard window Activity Monitor – All Processes, Secondary Actions: Raise
...
The focused UI element is 2 outline.
```

这个样本说明：

```text
部分 secondary action 是真正会改 UI 状态的动作（Expand / Collapse）。
也有一类更像窗口级命令（Raise），响应文本可能几乎不变。
```

## `scroll`

### Sample 1

Request

```json
{
  "app": "Activity Monitor",
  "element_index": "1",
  "direction": "down",
  "pages": 1
}
```

Response excerpt

```text
1 scroll area Secondary Actions: Scroll Up, Scroll Down
    2 outline Processes
        3 row (selectable) coreservicesd
        4 row (selectable) corebrightnessd
        ...
25 scroll bar (settable, float) 0.1371493880660984
    26 value indicator (settable, float) 0.1371493880660984
```

### Sample 2

Request

```json
{
  "app": "Activity Monitor",
  "element_index": "1",
  "direction": "up",
  "pages": 1
}
```

Response excerpt

```text
1 scroll area Secondary Actions: Scroll Up, Scroll Down
    2 outline Processes
        3 row (selectable) OrbStack Helper
        4 row (selectable) iTerm2
        ...
22 scroll bar (settable, float) 0
    23 value indicator (settable, float) 0
```

### Sample 3

Request

```json
{
  "app": "System Settings",
  "element_index": "5",
  "direction": "down",
  "pages": 1
}
```

Response excerpt

```text
5 scroll area Secondary Actions: Scroll Up, Scroll Down
    6 list Sidebar
        17 row (selectable) General
        18 row (selectable) Accessibility
        ...
        45 row (selectable) Printers & Scanners
46 scroll bar (settable, float) 0.9999999999999994
    47 value indicator (settable, float) 0.9999999999999994
```

## `set_value`

### Sample 1

Request

```json
{
  "app": "Activity Monitor",
  "element_index": "40",
  "value": "codex"
}
```

Response excerpt

```text
3 row (selectable) codex
...
14 row (selectable) Codex Computer Use
...
38 search text field (settable, string) codex
    39 button search
    40 button cancel
```

### Sample 2

Request

```json
{
  "app": "System Settings",
  "element_index": "3",
  "value": "privacy"
}
```

Response excerpt

```text
Window: "Apple Account", App: System Settings.
...
3 search text field (settable, string) privacy
    4 button Search
    5 button cancel
6 scroll area Secondary Actions: Scroll Up, Scroll Down
    7 list Sidebar
        9 row (selectable) Privacy & Security
        10 row (selectable) Allow applications to access Bluetooth
        ...
```

### Sample 3

Request

```json
{
  "app": "System Settings",
  "element_index": "3",
  "value": "screen"
}
```

Response excerpt

```text
Window: "Privacy & Security", App: System Settings.
...
3 search text field (settable, string) screen
    4 button Search
    5 button cancel
...
57 row (selectable) Privacy & Security
58 row (selectable) Allow applications to access the contents of your screen and audio through Remote Desktop
59 row (selectable) Allow applications to record your screen
```

## `press_key`

### Sample 1

Request

```json
{
  "app": "Activity Monitor",
  "key": "super+a"
}
```

Response excerpt

```text
38 search text field (settable, string) codex
...
Selected text: [codex]
```

### Sample 2

Request

```json
{
  "app": "Activity Monitor",
  "key": "super+a"
}
```

Response excerpt

```text
38 search text field (settable, string) Codex
...
Selected text: [Codex]
```

### Sample 3

Request

```json
{
  "app": "Activity Monitor",
  "key": "super+a"
}
```

Response excerpt

```text
27 search text field (settable, string) Computer Use
...
Selected text: [Computer Use]
```

## `type_text`

### Sample 1

Request

```json
{
  "app": "Activity Monitor",
  "text": "Codex"
}
```

Response excerpt

```text
38 search text field (settable, string) Codex
...
The focused UI element is 38 search text field.
```

### Sample 2

Request

```json
{
  "app": "Activity Monitor",
  "text": "Computer Use"
}
```

Response excerpt

```text
3 row (selectable) Codex Computer Use
...
27 search text field (settable, string) Computer Use
    28 button search
    29 button cancel
```

### Sample 3

Request

```json
{
  "app": "Activity Monitor",
  "text": "Helper"
}
```

Response excerpt

```text
2 outline Processes (showing 0-19 of 161 items)
    3 row (selectable) OrbStack Helper
    4 row (selectable) Figma Helper (Renderer)
    5 row (selectable) Figma Helper (GPU)
    ...
38 search text field (settable, string) Helper
```

## `drag`

### Sample 1

Request

```json
{
  "app": "Finder",
  "from_x": 158,
  "from_y": 373,
  "to_x": 220,
  "to_y": 373
}
```

Response excerpt

```text
24 splitter (disabled, settable, float) 185
...
170 pop up button Description: list view, Value: as List
```

这个样本里，`Finder` 左侧 sidebar 被拉宽，splitter 的 float 从 `133` 变成了 `185`。

### Sample 2

Request

```json
{
  "app": "Finder",
  "from_x": 220,
  "from_y": 373,
  "to_x": 158,
  "to_y": 373
}
```

Response excerpt

```text
24 splitter (disabled, settable, float) 133
...
171 radio button Description: icon view, Value: 0
172 radio button Description: list view, Value: 1
```

### Sample 3

Request

```json
{
  "app": "System Settings",
  "from_x": 231,
  "from_y": 466,
  "to_x": 260,
  "to_y": 466
}
```

Response excerpt

```text
83 splitter (disabled, settable, float) 215
...
57 row (selected) Privacy & Security
```

这个样本返回正常，但从文本上看没有产生明显的 split value 变化，说明纯坐标拖拽对起点定位比较敏感。

额外 no-op 倾向样本：

Request

```json
{
  "app": "Activity Monitor",
  "from_x": 1145,
  "from_y": 111,
  "to_x": 1145,
  "to_y": 262
}
```

Response excerpt

```text
22 scroll bar (settable, float) 0
    23 value indicator (settable, float) 0
...
38 search text field (settable, string) Helper
```

## 总结

从这轮实测可以先得到几个稳定结论：

```text
1. `list_apps` 是纯文本枚举接口，最简单。
2. `get_app_state` 是整个接口面的核心，会返回 element index、属性、secondary actions 和截图。
3. 大多数交互类工具在响应里都会把“最新的完整或近完整 UI 状态”再返回一遍，而不只是返回 `ok`。
4. `set_value` 比 `type_text` 更语义化，适合直接写 search/text field。
5. `press_key` 的响应里会带 `Selected text`，这对判断焦点和选区很有价值。
6. `drag` 只有坐标模式，没有 `element_index`，所以稳定性最依赖截图坐标选点。
7. app 名解析有边界：人类可读名不一定能直接命中，bundle id 也可能被安全策略拒绝。
```
