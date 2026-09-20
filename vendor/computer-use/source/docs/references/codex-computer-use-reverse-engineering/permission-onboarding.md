# Permission Onboarding

这个文档聚焦 Codex Computer Use 的系统权限引导体验，尤其是两类权限：

- `Accessibility`
- `Screen & System Audio Recording`

以及一个更具体的问题：它是否真的做了一个自定义窗口，引导用户把 `Codex Computer Use.app` 直接拖进授权区域，而不是只把用户丢到系统设置里自己找。

结论先写在前面：当前证据已经足够确认，官方实现不只是“检测权限然后打开 System Settings”。它内建了一套权限状态机和专门的 `SystemSettingsAccessoryWindow` 引导 UI；你看到的“可直接拖 app 进去”的体验，和二进制里出现的 `DraggableApplicationView`、`dragDelegate`、`ArrowWindow`、`TransitionOverlayWindow` 这些命名是高度一致的。

## 已观察事实

### 1. 权限 gating 是 service 内部的一等能力

`SkyComputerUseService` strings 里可见：

```text
ComputerUseIPCPermissionResult
ComputerUseIPCRequestRequiringSystemPermissions
ensureApplicationHasPermissions
Failed to request access to permission: %@
Failed to open System Settings for permission: %@
```

这说明权限不是外层随手做的前置检查，而是被并入了内部 request pipeline。

### 2. 官方明确跟踪两类核心权限

同一份 strings 里同时出现：

```text
accessibility
screenRecording
screen_recording
AccessibilityPermission
TCCDialogSystemPermission
Privacy_Accessibility
Privacy_ScreenCapture
```

结合此前真实 `System Settings` 样本里已经观察到的 `Screen & System Audio Recording` 页面，可以确认官方实现至少明确围绕：

- Accessibility
- Screen Recording / Screen & System Audio Recording

做权限 gating 和系统设置跳转。

### 3. 用户可见文案就是一套权限引导页，而不只是系统弹框

`SkyComputerUseService` 中可见一组连续的引导文案：

```text
Codex Computer Use needs these permissions to use apps on your Mac.
These permissions are only used when you ask Codex to perform tasks.
Allows Codex to access app interfaces
Codex uses screenshots to know where to click
COMPLETE IN SYSTEM SETTINGS
```

这说明主 app 里存在一层自己的权限说明界面。

### 4. 存在独立的权限窗口状态与回调

strings 里还能看到：

```text
permissionState
permissionsWindow
permissionsPending
permissionsNotGranted
onGrantAccessibility
onGrantScreenRecording
onOpenAccessibilitySettings
onOpenScreenRecordingSettings
```

这组命名已经不只是“去打开某个系统页”，而是说明它内部有：

- 权限窗口对象
- 权限状态机
- 分权限种类的 grant/open 回调

### 5. `SlimCore` 里有专门的 System Settings 引导窗口体系

当前已有的 strings 证据能直接看到：

```text
SystemSettingsAccessCoordinator
SystemSettingsAccessoryWindow
SystemSettingsAccessoryWindowView
SystemSettingsAccessoryWindowDragDelegate
SystemSettingsAccessoryTransitionOverlayWindow
SystemSettingsAccessoryTransitionOverlayReplicantWindow
SystemSettingsAccessoryTransitionBackground
ArrowWindow
```

这说明官方不是简单 `open x-apple.systempreferences:` 完事，而是围绕 System Settings 做了整套 accessory / overlay / transition UI。

### 6. “把 app 拖进去”的体验有直接命名证据

最关键的一组命名是：

```text
SystemSettingsAccessoryWindowView.DraggableApplicationView
dragDelegate
dragContinuation
draggable
ArrowWindow
```

这里的 `DraggableApplicationView` 几乎已经把实现意图写明白了：

- 会显示一个“可拖拽的应用视图”
- 配有 drag delegate
- 还有 arrow window 指示用户拖向目标区域

这和你观察到的“窗口里直接可以拖动 `Codex Computer Use.app` 进去，不需要自己去找 app”高度一致。

目前还能直接从 `Package_SlimCore.bundle` 导出一张和这套引导强相关的箭头资源：

- [hint-arrow.png](assets/extracted-2026-04-17/hint-arrow.png)

它对应的 asset 名就是 `HintArrow`，尺寸 `57x66`，与 `ArrowWindow` / `SystemSettingsAccessoryWindow` 这组命名非常吻合。

## 当前推断

### 1. 权限流程大概率分成两层

当前最合理的结构是：

- `SkyComputerUseService`
  - 检查权限状态
  - 决定当前缺哪项权限
  - 打开对应的 System Settings 页面
  - 管理 permission window 生命周期
- `SlimCore`
  - 提供具体的 accessory window / overlay / drag UI
  - 用箭头、过渡遮罩和 draggable app view 引导用户完成系统设置里的最后一步

### 2. 你看到的拖拽窗口更像官方自定义引导壳，不是系统原生默认界面

原因很直接：

- 系统设置本身不会暴露 `DraggableApplicationView` 这种命名给第三方 app。
- `ArrowWindow`、`TransitionOverlayWindow`、`permissionsWindow` 都属于官方 bundle 的内部符号。
- 文案 `COMPLETE IN SYSTEM SETTINGS` 暗示是“本 app 先解释，再把最后一步交给系统设置”。

所以更合理的解释是：

- 真实授权最终仍然发生在 macOS 的 TCC / System Settings 里；
- 但官方 app 在旁边叠加了一层辅助窗口，减少用户自己找入口、找 app、找拖拽目标的成本。

### 3. Accessibility 的拖拽引导证据比 Screen Recording 更强

从语义上看：

- `Accessibility` 页面本来就更符合“把 app 拖进允许列表”的交互。
- `Screen Recording` / `Screen & System Audio Recording` 常见的是切换条目或确认重新打开 app。

因此当前更强的推断是：

- `DraggableApplicationView` 这套交互至少服务于 Accessibility 授权；
- 是否也完全同样用于 Screen Recording，还需要额外动态观察才能百分百确认。

## 为什么这套体验很重要

这说明官方在产品层面并不满足于“把用户踢去系统设置自己想办法”。它专门补了一层：

- 权限解释
- 精准跳页
- 拖拽引导
- 回到 app 后的状态收敛

对开源实现来说，这个点很关键，因为它直接影响首轮授权成功率和用户挫败感。

## 还没完全确认的点

- 当前没有现场抓到权限 onboarding 窗口的运行时截图，因为本机此时权限已经授予。
- `DraggableApplicationView` 是否仅用于 Accessibility，还是也参与了 Screen Recording 引导，暂时仍是推断。
- 还没有从 `Assets.car` 中把这套权限 UI 的具体视觉资源完整提取出来。

## 对开源实现的启发

- 不要只做“检查失败就提示用户手动去系统设置”。
- 应该把权限流程单独建模成状态机，而不是散落在工具调用失败分支里。
- 最好提供面向具体系统页面的辅助窗口，尤其是 Accessibility 这种步骤多、路径深的授权流程。
- 把“打开系统设置”和“最后一步如何完成”拆开设计，体验会比纯文案提示好很多。
