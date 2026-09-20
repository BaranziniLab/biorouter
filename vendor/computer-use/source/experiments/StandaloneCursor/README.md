# Standalone Cursor

这个目录承载一条新的独立 cursor demo 线路，目标不是继续堆调参 UI，而是直接把 `scripts/cursor-motion-re/official_cursor_motion.py` 里已经收敛出来的重建结果搬成一个可运行的 Swift app。

## 目标边界

- 直接复用 Python 脚本里已经确认的几何和 timing 核心：
  - `2` 条 base candidate + `3 x 3 x 2` arched candidate
  - `sample(progress)` 与 `measure(...)`
  - `prefer in-bounds, then lowest score`
  - `response=1.4`、`dampingFraction=0.9`、`dt=1/240` 的 raw spring timeline
- 刻意不引入当前还没恢复出来的 wall-clock duration 映射。
- 刻意不复用 `CursorMotion` 那套更偏实验性的 visual dynamics / knob 调参结构。

## 运行方式

```bash
swift run StandaloneCursor
```

## 当前交互

- 拖动 `START` / `END` handle，实时重算 `20` 条候选路径。
- 右侧面板会列出全部 candidates、score、length、turn 和 in-bounds 状态。
- 默认按 Python 脚本同一套策略自动选路，也可以手动锁定某一条 candidate。
- `Replay` 会按 raw spring timeline 重放当前选中路径。

## 适用场景

- 想看 Python 重建逻辑换成 Swift 之后的实际轨迹和时序表现。
- 想快速核对候选路径池、score 和 endpoint lock / close-enough 时间，而不是继续调视觉手感。
- 想和 `CursorMotion` 做对照，区分“脚本级 binary lift”与“更自由的实验 demo”。
