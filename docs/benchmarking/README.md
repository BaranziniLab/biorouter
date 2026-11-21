# Benchmarking

This folder holds Biorouter's plans and results for comparing its agent harness with other
harnesses, such as Codex and Claude Code, on external benchmarks. The first one is
BioRouterBench, the bioinformatics benchmark of the Biorouter manuscript. Its tasks, harness code
and run data live in the private manuscript repository; what lives here is how a run is set up,
what it measures, how its numbers are compared, and what came out.

Come here before you configure Biorouter for a benchmark run, or before you quote a benchmark
number, so that the number carries the configuration it was measured under.

| Document | What it covers |
| --- | --- |
| [BioRouterBench stock profile rerun plan](biorouterbench-stock-profile-rerun-plan.md) | The plan for rerunning BioRouterBench 1.1.0 with Biorouter in its shipped configuration beside Codex and Claude Code, and for comparing Biorouter 1.94.5 with 1.94.6 (issue #541): arms, Biorouter profiles, models, tasks, the EC2 environment and its credential handling, relay and classifier changes, stages and commands, metrics, statistics, budget, risks and acceptance criteria. Proposed; not yet run. |

What will land here next: the results of that rerun, as a dated document beside the plan, and a
reference for the Biorouter benchmark profiles once a second run reuses them. When the rerun
concludes, the plan and its results move to [`history/`](../history/README.md) as a campaign,
and anything still in use, such as the profile reference, stays here.

**Boundaries.** The `biorouter bench` command (a general model leaderboard tool) is documented in
the [`biorouter-bench` crate's README](../../crates/biorouter-bench/README.md). Speed and memory
measurements of the binaries themselves are in [`benchmarks/`](../../benchmarks/bench.sh) and
[the June 2026 performance review](../history/performance-2026-06/README.md). Studies of how other
agents are designed, without running them on tasks, are in [`research/`](../research/README.md).

## Related documentation

- [Issue #534](https://github.com/BaranziniLab/biorouter/issues/534): the BioRouterBench 1.1.0
  harness review, which the rerun plan answers.
- [Agent loop](../agent-loop/README.md): how the harness being benchmarked works.
- [Research](../research/README.md): studies of other coding agents' designs.
