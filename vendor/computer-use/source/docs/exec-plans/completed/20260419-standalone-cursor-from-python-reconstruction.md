# 基于 Python 重建脚本实现新的 StandaloneCursor

## 目标

新增一条不依赖现有 `CursorMotion` 的独立 Swift demo 线路，把 `scripts/cursor-motion-re/official_cursor_motion.py` 已经收敛出来的候选路径、score、selection policy 和 raw spring timeline 直接搬进一个可运行的 `swift run StandaloneCursor` app。

## 范围

- 包含：
  - 新增 `StandaloneCursor` executable target。
  - 新增独立 support module，承载 Python 脚本对应的 Swift 数据模型、候选生成、评分与 timeline。
  - 新增最小可交互 app UI，用于拖动起终点、选 candidate、重放路径。
  - 补充 README、架构文档和 history。
- 不包含：
  - 不修改现有 `CursorMotion` 的代码和 UI 逻辑。
  - 不把这条 demo 直接接回主 `SoftwareCursorOverlay`。
  - 不宣称已经恢复官方 wall-clock duration 映射。

## 背景

- 相关文档：
  - `docs/ARCHITECTURE.md`
  - `docs/REPO_COLLAB_GUIDE.md`
  - `scripts/cursor-motion-re/README.md`
- 相关代码路径：
  - `scripts/cursor-motion-re/official_cursor_motion.py`
  - `experiments/StandaloneCursor/`
  - `Package.swift`
- 已知约束：
  - 当前 worktree 里 `CursorMotion` 相关文件已经处于脏状态，应避免继续踩那条线。
  - 这次目标是“更贴 Python 脚本的独立 viewer”，不是再堆一版可调参数实验室。

## 风险

- 风险：把 Python 脚本里“已确认的核心”和“仍未恢复的 duration 映射”混在一起。
- 缓解方式：UI 和文档里显式写明只复用 raw spring timeline，不假装已经恢复 wall-clock duration。

- 风险：与当前 `CursorMotion` 的本地改动发生冲突。
- 缓解方式：新建完全独立的 target、源码目录和测试目录。

## 里程碑

1. 新 target 和 support model 边界收敛。
2. Swift app 与交互视图实现。
3. 验证、文档与 history 收尾。

## 验证方式

- 命令：
  - `swift build --product StandaloneCursor`
  - `swift test --filter StandaloneCursorSupportTests`
  - `swift run StandaloneCursor`
- 手工检查：
  - 拖动 `START` / `END`，确认候选路径和选中 candidate 会更新。
  - 点击右侧 candidate，确认能切到手动锁定并重放。
- 观测检查：
  - UI 中能看到 endpoint lock / close-enough 时间以及 raw progress。
  - 文档明确区分 `StandaloneCursor` 与 `CursorMotion` 的职责。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-19：不继续改当前 `CursorMotion`，改为新增 `StandaloneCursor` target，避免与现有脏 worktree 冲突。
- 2026-04-19：support model 直接按 Python 脚本命名和结构翻译，优先保证候选路径、评分和 timeline 的可对照性。
- 2026-04-19：新 app 刻意不做 speculative 的视觉 pose dynamics，只展示 binary-guided path pool 与 raw spring playback。
