# Cursor Motion RE Scripts

这个目录承载和官方 `Codex Computer Use.app` cursor motion 逆向相关的独立脚本，不依赖 `CursorMotion`，也不接入主运行时。

当前脚本优先做两件事：

- 从官方 bundled `SkyComputerUseService` 里提取 motion 相关的 Swift 类型、字段和常量。
- 基于已经从二进制直接确认的 `CursorMotionPath.sample(progress)`、`CursorMotionPathMeasurement`、`CursorMotionPath/Segment` 布局、`0x10005fd98` 候选几何，以及已经坐实的 `SpringAnimation -> VelocityVerletSimulation` timing 证据，输出一版 binary-lifted 的候选路径和分析结果。

## 文件

- `official_cursor_motion.py`
  - 逆向辅助模块，包含最小 Mach-O section 解析、Swift field metadata 恢复、常量表读取，以及独立的 path / measurement / candidate demo 实现。
- `reconstruct_cursor_motion.py`
  - CLI 入口。

## 用法

查看官方 binary 中恢复出的 motion 类型、字段、常量与候选系数表：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py inspect
```

对给定起终点生成候选路径，并输出 JSON：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py demo \
  --start 100 120 \
  --end 720 380 \
  --bounds 0 0 1280 800 \
  --samples 32 \
  --pretty
```

不带 `--bounds` 时，`stays_in_bounds` 会退化为 `true`，只计算几何量。

如需把所有候选的完整 path 和 sample 一次性打出来，再加：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py demo \
  --start 100 120 \
  --end 720 380 \
  --bounds 0 0 1280 800 \
  --samples 32 \
  --include-all-candidates \
  --pretty
```

分析视频里那 5 个 slider 在 shipping bundle 中是否还有直接证据，并输出它们和当前 binary-confirmed 几何 / spring 量的敏感性分析：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py slider-study \
  --start 220 440 \
  --end 860 260 \
  --bounds 0 0 1120 760 \
  --pretty
```

## 输出说明

- `inspect`：
  - 输出从官方 binary 中恢复出的 motion 相关类型和字段。
  - 输出当前版本 bundled app 中提取出的 data-section 常量、候选系数表，以及从反汇编直接确认的 scoring / layout / piecewise 几何常量。
  - 额外输出 binary-confirmed 的 timing 证据：`CloseEnoughConfiguration` / `CursorNextInteractionTiming` / `SpringParameters` / `AnimationDescriptor` / `Transaction` / `VelocityVerletSimulation.Configuration` 的字段关系，以及 cursor path animation 的 `response=1.4`、`dampingFraction=0.9`、`dt=1/240`、`idleVelocityThreshold=28800`。
  - `VelocityVerlet` 的 `stiffness` / `drag` 公式和单步 `VelocityVerlet` 更新顺序现在已经按二进制直译。
  - 还会附带 `0x1005761bc` / `0x1005934b0` 的 finish predicate 证据块，明确哪些是 confirmed control flow，哪些仍然只是字段命名推断。
- `demo`：
  - 默认输出 `candidate_summaries` 和 `chosen_candidate`，避免一次性打印全部候选采样点。
  - `--include-all-candidates` 会额外输出所有候选的完整控制点、measurement 和采样点。
  - `sample(progress)` 与 `measure_path()` 是从函数控制流直接 lift 出来的实现。
  - candidate score、in-bounds 优先策略、`CursorMotionPath/Segment` 布局，以及 `20` 条候选几何都已经按当前 bundled binary 直译。
  - 时间轴输出会额外标出 `raw_progress_first_ge_target_*`、`first_endpoint_lock_*` 和 `close_enough_first_*`，方便对照 spring progress、可见端点锁定和 close-enough 判定。
  - 当前仍未完全恢复的是 duration / wall-clock timing、`0x1005934b0` 第二段里几个泛型 buffer 的精确语义命名，以及调用前那层 runtime bounds 自动发现。
  - `first_endpoint_lock_*` 依赖一个已确认前提：`sample(progress)` 会 clamp 到 `0...1`；但它和 `SpringAnimation` finished optional-return 的最终联动，当前仍按“多段已确认证据拼接出的 inference”标注。
  - 当前输出的 `speed_units_per_progress` 是几何速度，不是带真实 duration 的时间速度；duration 仍在继续逆向。
- `slider-study`：
  - 先扫描 shipping bundle，确认 `START HANDLE` / `END HANDLE` / `ARC SIZE` / `ARC FLOW` 这些完整 phrase 是否还存在。
  - 再把这 5 个 knob 分别映射到当前已经 binary-confirmed 的 `startControl` / `endControl` / `arc*` / `SpringParameters` 相关量，并输出 baseline 与扰动后的 chosen candidate / best arched candidate / endpoint-lock timing 变化。
  - 输出里会明确区分“release bundle phrase evidence”和“基于 binary-confirmed 几何做的 slider mapping inference”。
