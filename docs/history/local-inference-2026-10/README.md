# Local inference validation campaign

> **What this is.** The October 2026 investigation and validation record for native Ollama
> context/tool-call handling and the managed Llama Server integration.
> **Status:** Historical record — validation concluded 2026-10-12 UTC (2026-10-11 PDT);
> implementation is in unmerged [PR #393](https://github.com/BaranziniLab/biorouter/pull/393).
> **Audience:** maintainers tracing provider behavior, evidence and retained limitations.

The validation campaign finished on 2026-10-12 UTC (2026-10-11 PDT). Its records preserve actual
source-scoped checks, model-quality/workflow failures and incomplete or excluded coverage;
a finished validation campaign does not mean every model task passed. PR #393 remains
unmerged, and no released version or deployment is claimed. Final-head CI and durable
archive verification remain separate delivery requirements recorded in the validation report.

| Record | What it holds |
| --- | --- |
| [Validation](local-inference-validation.md) | Actual local/CI/live outcomes, source and binary scope, retained failures, closed roster/database proof and the expected archive-completion procedure. |
| [Investigation and handoff](local-inference-handoff.md) | Root cause, implementation decisions, historical reports, safety limits, evidence lineage and delivery requirements. |

## Related documentation

- [Providers index](../../providers/README.md)
- [Ollama integration](../../providers/ollama/README.md)
- [Managed Llama Server catalog](../../providers/llama-server/model-catalog.md)
