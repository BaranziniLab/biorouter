# Background Click Free Tooling Workflow

这份文档记录一次用免费工具替代 IDA Pro 研究官方 bundled `computer-use` 后台点击路径的方法论。它不是要把一次性 Ghidra project、反汇编大文件或临时 Swift 原型提交进仓库，而是让后续开发者或 AI 可以按同样步骤重新生成本地研究目录。

## 适用场景

当需要研究官方闭源 `Codex Computer Use.app` 的鼠标点击、后台窗口投递、`CGEvent` 字段或 AppKit / AX 交互路径时，优先按本文新建一个本地 `research/<topic>-<date>/` 目录。仓库级 `.gitignore` 已忽略 `research/`，避免把大体积二进制分析产物带进提交。

## 工具链

免费工具组合：

```bash
brew install ghidra radare2
python3 -m pip install --user frida-tools lief capstone
```

常用系统工具：

```bash
xcrun -f otool
xcrun -f nm
xcrun -f swift-demangle
xcrun -f llvm-objdump
codesign --help
```

如果 Ghidra headless 找不到 Java，显式设置：

```bash
export JAVA_HOME="$(brew --prefix openjdk@21)/libexec/openjdk.jdk/Contents/Home"
```

Ghidra headless 入口通常是：

```bash
"$(brew --prefix ghidra)/libexec/support/analyzeHeadless"
```

## 目标定位

默认从 bundled plugin 中取目标二进制。不要在文档里写机器绝对路径；本地运行时用环境变量拼出来：

```bash
CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"
APP="$CODEX_HOME/plugins/cache/openai-bundled/computer-use/1.0.755/Codex Computer Use.app"
SERVICE="$APP/Contents/MacOS/SkyComputerUseService"
CLIENT="$APP/Contents/SharedSupport/SkyComputerUseClient.app/Contents/MacOS/SkyComputerUseClient"
```

先确认 bundle、签名和 Mach-O 类型：

```bash
file "$SERVICE" "$CLIENT"
codesign -dv --verbose=4 "$APP" > raw/codesign-service-app.txt 2>&1
codesign -dv --verbose=4 "$APP/Contents/SharedSupport/SkyComputerUseClient.app" > raw/codesign-client-app.txt 2>&1
```

## 静态信息采集

建议先把原始输出落到 `research/<topic>/raw/`，方便 AI 后续 grep 和引用：

```bash
mkdir -p raw
nm -u "$SERVICE" > raw/nm-u-service.txt
nm -u "$CLIENT" > raw/nm-u-client.txt
otool -L "$SERVICE" > raw/otool-L-service.txt
otool -Iv "$SERVICE" > raw/otool-Iv-service.txt
rabin2 -I "$SERVICE" > raw/rabin2-I-service.txt
rabin2 -i "$SERVICE" > raw/rabin2-imports-service.txt
rabin2 -zz "$SERVICE" > raw/rabin2-strings-service.txt
strings "$SERVICE" | swift-demangle > raw/strings-demangled-service.txt
```

对后台点击方向，优先搜索这些关键词：

```bash
rg "mouseEventWithType|eventWithCGEvent|CGEvent|buttonNumber|clickCount|windowNumber|AXUIElement|CGWindowList|dlopen|dlsym" raw
```

高价值命中通常包括：

- `mouseEventWithType:location:modifierFlags:timestamp:windowNumber:context:eventNumber:clickCount:pressure:`
- `eventWithCGEvent:`
- `AXUIElementCopyElementAtPosition`
- `AXUIElementPerformAction`
- `AXUIElementSetAttributeValue`
- `CGWindowListCopyWindowInfo`
- `CGWindowListCreateDescriptionFromArray`
- `dlopen` / `dlsym`

## Ghidra Headless 导出策略

不要手工在 GUI 里逐个点函数。写一个 headless script，按符号名和字符串命中收集 candidate functions，再批量 decompile 到 `ghidra-service/`。

推荐脚本能力：

- 输入关键词列表，例如 `AXUIElement`、`CGWindowList`、`NSEvent`、`CGEvent`、`mouseEventWithType`、`windowNumber`。
- 遍历 symbol table、defined strings 和 references。
- 将命中函数 decompile 成独立 `.c` 文件。
- 额外支持按地址导出，方便二次追踪 wrapper / resolver。

运行形态：

```bash
mkdir -p ghidra-project ghidra-service tools
"$(brew --prefix ghidra)/libexec/support/analyzeHeadless" \
  "$PWD/ghidra-project" ComputerUseService \
  -import "$SERVICE" \
  -scriptPath "$PWD/tools" \
  -postScript FocusExport.java "$PWD/ghidra-service"
```

如果已经知道关键地址，再用单独脚本导出：

```bash
"$(brew --prefix ghidra)/libexec/support/analyzeHeadless" \
  "$PWD/ghidra-project" ComputerUseService \
  -process SkyComputerUseService \
  -scriptPath "$PWD/tools" \
  -postScript ExportAddresses.java "$PWD/ghidra-service/address-decompiled" \
  10050d404 100569a5c 1000ae328 1000ae398 1000b8830
```

## 后台点击核心判断

本轮已经验证出的关键路径如下。后续 AI 重新生成代码时，应以这个调用形状为优先方向：

1. 用 `CGWindowListCopyWindowInfo` 找到目标进程的 layer 0 窗口、bounds 和 `CGWindowID`。
2. 用 `NSEvent.mouseEvent(...)` 创建 down/up 事件，`windowNumber` 传目标 `CGWindowID`。
3. 从 `NSEvent.cgEvent` 取出底层 `CGEvent`。
4. 设置字段：
   - field `3`: button number，也就是公开的 `kCGMouseEventButtonNumber`
   - field `7`: 观察值为 `3`
   - field `91`: window under pointer
   - field `92`: event target window
5. 设置 screen-space `CGEvent.location`。
6. 把 screen point 转成 window-local point。
7. 调用私有符号 `CGEventSetWindowLocation(event, localPoint)`。
8. 用 `CGEvent.postToPid(pid)` 定向投递到目标进程。

在 AppKit 测试目标上，`CGEventSetWindowLocation` 是关键对照项：保留它时后台窗口能收到 `mouseDown/mouseUp`；跳过它时没有新点击到达。官方二进制的某个后台分支会把 flags 置为 `0x100000`，但简单 AppKit fixture 上它不是事件到达的必要条件。

## 最小 Swift 复现方向

让 AI 生成本地原型时，建议建一个 Swift package，包含两个 executable target：

- `BackgroundClickProbe`
  - 参数支持 `--pid`、`--bundle-id`、`--app`、`--window-id`、`--point`、`--dry-run`。
  - 用 `dlsym(RTLD_DEFAULT, "CGEventSetWindowLocation")` 解析私有符号。
  - 输出 JSON，包含 pid、window id、bounds、screen point、local point、flags 和是否 dry run。
- `EchoApp`
  - 一个最小 AppKit 窗口。
  - override `mouseDown` / `mouseUp`。
  - 同时打印 stdout 并写 `/tmp/bgclick-echoapp.log`，方便用 `open` 或后台进程验证。

验证矩阵至少覆盖：

```bash
swift build
ECHOAPP_LOG=/tmp/bgclick-echoapp.log .build/debug/EchoApp
.build/debug/BackgroundClickProbe --app EchoApp
.build/debug/BackgroundClickProbe --app EchoApp --no-background-flag
.build/debug/BackgroundClickProbe --app EchoApp --skip-window-location
.build/debug/BackgroundClickProbe --app EchoApp --nsevent-location local
tail -n 20 /tmp/bgclick-echoapp.log
```

期望看到目标在 `active=false`、`key=false` 时仍收到 `mouseDown` / `mouseUp`。如果默认命令失败，优先检查：

- `AXIsProcessTrusted()` 是否为 true。
- `dlsym` 是否能解析 `CGEventSetWindowLocation`。
- `CGWindowID` 是否来自目标进程的 layer 0 on-screen window。
- `window-local` 坐标是否按 `screenPoint - windowBounds.origin` 计算。
- 是否误用了全局 HID post，而不是 `postToPid`。

## 提交边界

不要提交这些内容：

- `research/` 下的一次性 Ghidra project。
- 大体积 `llvm-objdump` / `rabin2 -zz` / strings 原始输出。
- 临时 Swift package 的 `.build/`。
- 从官方 bundle 直接导出的二进制或大资源，除非明确放到 `docs/references/.../assets/` 并配合 Git LFS 规则。

可以提交这些内容：

- 经过整理的结论文档。
- 小型、通用、可复跑的 Ghidra script 或采集脚本。
- 已经产品化并进入主 runtime / smoke 的开源实现代码。

## 已知限制

- Ghidra 伪代码不是源码，Swift async、ObjC message send 和动态 resolver 处需要靠调用形状、字符串和运行验证共同判断。
- `CGEventSetWindowLocation` 是私有 API，只适合研究和本地对照，不应直接作为发布产品依赖。
- 行为依赖 macOS 版本、TCC 权限、目标 App toolkit 和签名/宿主环境。
