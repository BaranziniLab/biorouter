# Subworkflows

> **What this is.** How one workflow delegates sub-tasks to other workflow files — the `sub_workflows` schema, how parameters reach a subworkflow, the isolation rules, and three worked pipelines.
> **Status:** Current — but the feature itself is experimental and in active development; behaviour and configuration may change in future releases.
> **Audience:** end users composing multi-step workflows.

> **Warning.** Subworkflows are an experimental feature in active development. Behaviour and configuration may change in future releases. Read this before building anything you depend on.

Subworkflows are workflows that are used by another workflow to perform specific tasks. They enable:

- **Multi-step workflows** — break complex tasks into distinct phases with specialized expertise
- **Reusable components** — create common tasks that can be used in various workflows

## How subworkflows work

The "main workflow" registers its subworkflows in the `sub_workflows` field, which contains the following fields:

- `name`: Unique identifier for the subworkflow, used to generate the tool name
- `path`: File path to the subworkflow file (relative or absolute)
- `values`: (Optional) Pre-configured parameter values that are always passed to the subworkflow

When the main workflow is run, Biorouter generates a tool for each subworkflow that:

- Accepts parameters defined by the subworkflow
- Executes the subworkflow in a separate session with its own context
- Returns output to the main workflow

Subworkflow sessions run in isolation — they don't share conversation history, memory, or state with the main workflow or other subworkflows. Additionally, subworkflows cannot define their own subworkflows (no nesting allowed).

The complete `sub_workflows` field schema, including `sequential_when_repeated` and `description`, is in the [workflow schema reference](workflow-schema-reference.md#subworkflows).

### Parameter handling

Parameters received by subworkflows can be used in prompts and instructions using `{{ parameter_name }}` syntax. Subworkflows receive parameters in two ways:

1. **Pre-set values**: Fixed parameter values defined in the `values` field are automatically provided and cannot be overridden at runtime
2. **Context-based parameters**: The AI agent can extract parameter values from the conversation context, including results from previous subworkflows

Pre-set values take precedence over context-based parameters. If both the conversation context and `values` field provide the same parameter, the `values` version is used.

> **Tip.** Use the `indent()` filter to maintain valid YAML format when passing multi-line parameter values to subworkflows, for example: `{{ content | indent(2) }}`. See [Template support](workflow-schema-reference.md#template-support) for more details.

### Monitors on a subworkflow's subagent

A subworkflow runs as a subagent of the main workflow's chat, so it can be watched by decision-model monitors. When the subworkflow's file declares `decision_settings` ([Supervision](workflow-schema-reference.md#supervision-decision_settings-and-orchestration_plan)), the subagent spawned from it starts with those monitors, as the person's, after the copies of the parent chat's monitors set to also watch subagents. Its default decision model becomes the subagent's when the parent chat has none. Each monitor's model is checked against the parent chat before anything starts, and a monitor the chat may not use refuses the spawn ("Monitor 'Off task' could not be put on the subagent: … Nothing was started."). The monitors follow the rules in [Monitors on subagents](../agent-loop/decision-models.md#monitors-on-subagents).

A subagent cannot run phases, because every workspace tool is refused to it. So a subworkflow whose file declares an `orchestration_plan` is refused when it would be spawned: "Could not spawn subworkflow "verify": subworkflow `verify` declares an `orchestration_plan`, and a subagent cannot run phases: workspace tools are refused to subagents. Remove the block from that file. Nothing was started."

## Examples

The three examples below each illustrate one pattern. They are named after their domain in the YAML, so the mapping is:

| Pattern | Example workflow |
|---|---|
| Sequential processing | `code-review-pipeline.yaml` |
| Conditional processing | `smart-analyzer.yaml` |
| Context-based parameter passing | `drug-repurposing.yaml` |

### Sequential processing: the code review pipeline

This Code Review Pipeline example shows a main workflow that uses two subworkflows to perform a comprehensive code review. The main workflow's instructions fix the order: security first, then quality.

**Usage:**
```bash
biorouter run --workflow code-review-pipeline.yaml --params repository_path=/path/to/repo
```

**Main workflow:**

```yaml
# code-review-pipeline.yaml
version: "1.0.0"
title: "Code Review Pipeline"
description: "Automated code review using subworkflows"
instructions: |
  Perform a code review using the available subworkflow tools.
  Run security analysis first, then code quality analysis.

parameters:
  - key: repository_path
    input_type: string
    requirement: required
    description: "Path to the repository to review"

sub_workflows:
  - name: "security_scan"
    path: "./subworkflows/security-analysis.yaml"
    values:
      scan_level: "comprehensive"
  
  - name: "quality_check"
    path: "./subworkflows/quality-analysis.yaml"

extensions:
  - type: builtin
    name: developer
    timeout: 300
    bundled: true

prompt: |
  Review the code at {{ repository_path }} using the subworkflow tools.
  Run security scan first, then quality analysis.
```

**Subworkflow `security_scan`** — note that `scan_level` is supplied by the main workflow's `values`, so its `default` never applies here:

```yaml
  # subworkflows/security-analysis.yaml
  version: "1.0.0"
  title: "Security Scanner"
  description: "Analyze code for security vulnerabilities"
  instructions: |
    You are a security expert. Analyze the provided code for security issues.
    Focus on common vulnerabilities like SQL injection, XSS, and authentication flaws.

  parameters:
    - key: repository_path
      input_type: string
      requirement: required
      description: "Path to the code to analyze"
    
    - key: scan_level
      input_type: string
      requirement: optional
      default: "standard"
      description: "Depth of security scan (basic, standard, comprehensive)"

  extensions:
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Perform a {{ scan_level }} security analysis on the code at {{ repository_path }}.
    Report any security vulnerabilities found with severity levels and recommendations.
```

**Subworkflow `quality_check`** — takes only `repository_path`, which the agent carries over from the main workflow's context:

```yaml
  # subworkflows/quality-analysis.yaml
  version: "1.0.0"
  title: "Code Quality Analyzer"
  description: "Analyze code quality and best practices"
  instructions: |
    You are a code quality expert. Review code for maintainability, 
    readability, and adherence to best practices.

  parameters:
    - key: repository_path
      input_type: string
      requirement: required
      description: "Path to the code to analyze"

  extensions:
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Analyze the code quality at {{ repository_path }}.
    Check for code smells, complexity issues, and suggest improvements.
```

> **Tip.** For faster execution when subworkflows are independent, multiple subworkflows can be executed concurrently — set `sequential_when_repeated: false` on the subworkflow entry.

### Conditional processing: the smart project analyzer

This Smart Project Analyzer example shows conditional logic that chooses between different subworkflows based on analysis. Both subworkflows are registered, but the instructions tell the agent to run only one.

**Usage:**
```bash
biorouter run --workflow smart-analyzer.yaml --params repository_path=/path/to/project
```

**Main workflow:**

```yaml
# smart-analyzer.yaml
version: "1.0.0"
title: "Smart Project Analyzer"
description: "Analyze project and choose appropriate processing based on type"
instructions: |
  First examine the repository to determine the project type (web app, CLI tool, library, etc.).
  Based on what you find:
  - If it's a web application, use the web_security_audit subworkflow
  - If it's a CLI tool or library, use the api_documentation subworkflow
  Only run one subworkflow based on your analysis.

parameters:
  - key: repository_path
    input_type: string
    requirement: required
    description: "Path to the repository to analyze"

sub_workflows:
  - name: "web_security_audit"
    path: "./subworkflows/web-security.yaml"
    values:
      check_cors: "true"
      check_csrf: "true"
  
  - name: "api_documentation"
    path: "./subworkflows/api-docs.yaml"
    values:
      format: "markdown"

extensions:
  - type: builtin
    name: developer
    timeout: 300
    bundled: true

prompt: |
  Analyze the project at {{ repository_path }} and determine its type.
  Then run the appropriate subworkflow tool based on your findings.
```

**Subworkflow `web_security_audit`** — the `{% if %}` blocks in the prompt branch on the `values` the main workflow pinned to `"true"`:

```yaml
  # subworkflows/web-security.yaml
  version: "1.0.0"
  title: "Web Security Auditor"
  description: "Security audit for web applications"
  instructions: |
    You are a web security specialist. Audit web applications for 
    security vulnerabilities specific to web technologies.

  parameters:
    - key: repository_path
      input_type: string
      requirement: required
      description: "Path to the web application code"
    
    - key: check_cors
      input_type: string
      requirement: optional
      default: "false"
      description: "Whether to check CORS configuration"
    
    - key: check_csrf
      input_type: string
      requirement: optional
      default: "false"
      description: "Whether to check CSRF protection"

  extensions:
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Perform a web security audit on {{ repository_path }}.
    {% if check_cors == "true" %}Check CORS configuration for security issues.{% endif %}
    {% if check_csrf == "true" %}Verify CSRF protection is properly implemented.{% endif %}
    Focus on web-specific vulnerabilities like XSS, authentication flaws, and session management.
```

**Subworkflow `api_documentation`** — the alternative branch, taking its output `format` from the main workflow's `values`:

```yaml
  # subworkflows/api-docs.yaml
  version: "1.0.0"
  title: "API Documentation Generator"
  description: "Generate documentation for APIs and libraries"
  instructions: |
    You are a technical writer specializing in API documentation.
    Create comprehensive documentation for code libraries and APIs.

  parameters:
    - key: repository_path
      input_type: string
      requirement: required
      description: "Path to the code to document"
    
    - key: format
      input_type: string
      requirement: optional
      default: "markdown"
      description: "Output format for documentation (markdown, html, rst)"

  extensions:
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Generate {{ format }} documentation for the code at {{ repository_path }}.
    Include API endpoints, function signatures, usage examples, and installation instructions.
    Focus on making it easy for developers to understand and use this code.
```

### Context-based parameter passing: the drug repurposing explorer

This Drug Repurposing example shows how subworkflows can receive parameters from conversation context, including results from previous subworkflows. Neither subworkflow entry declares `values` — every parameter is inferred by the agent from the conversation.

**Usage:**
```bash
biorouter run --workflow drug-repurposing.yaml
```

**Main workflow** — the drug name appears only in the prompt text, never as a declared parameter:

```yaml
# drug-repurposing.yaml
version: "1.0.0"
title: "Drug Repurposing Explorer"
description: "Fetch known drug targets, then suggest repurposing candidates"
instructions: |
  Explore repurposing opportunities by first retrieving the drug's known targets, then suggesting candidate diseases based on those targets.

prompt: |
  Explore repurposing opportunities for metformin by first retrieving its known targets, then suggesting candidate diseases based on the targets we receive.

sub_workflows:
  - name: target_data
    path: "./subworkflows/target-data.yaml"
    # No values - drug_name parameter comes from prompt context
  
  - name: repurposing_suggestions
    path: "./subworkflows/repurposing-suggestions.yaml"
    # known_targets parameter comes from conversation context

extensions:
  - type: builtin
    name: developer
    timeout: 300
    bundled: true
```

**Subworkflow `target_data`** — runs first, and adds a `stdio` extension of its own that the main workflow does not have:

```yaml
  # subworkflows/target-data.yaml
  version: "1.0.0"
  title: "Drug Target Collector"
  description: "Fetch known protein targets for a drug"
  instructions: |
    You are a pharmacology data specialist. Gather the known molecular
    targets for the drug, including target genes and mechanism of action.

  parameters:
    - key: drug_name
      input_type: string
      requirement: required
      description: "Drug name to look up targets for"

  extensions:
    - type: stdio
      name: spoke
      cmd: uvx
      args:
        - mcp_spoke@latest
      timeout: 300
      description: "Query the SPOKE knowledge graph for drug targets"
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Retrieve the known protein targets for {{ drug_name }}.
    Include target gene symbols, mechanism of action,
    and any relevant pathway context.
```

**Subworkflow `repurposing_suggestions`** — runs second, consuming the first subworkflow's output as `known_targets`:

```yaml
  # subworkflows/repurposing-suggestions.yaml
  version: "1.0.0"
  title: "Repurposing Candidate Recommender"
  description: "Suggest candidate diseases based on drug targets"
  instructions: |
    You are a translational research expert. Recommend candidate diseases
    a drug might be repurposed for, based on its known molecular targets.

  parameters:
    - key: known_targets
      input_type: string
      requirement: required
      description: "Known drug targets to base recommendations on"

  extensions:
    - type: builtin
      name: developer
      timeout: 300
      bundled: true

  prompt: |
    Based on these drug targets: {{ known_targets }}, 
    suggest candidate diseases for repurposing along with the
    biological rationale and supporting evidence for each.
```

In this example:

- The `target_data` subworkflow gets the drug name from the prompt context (the AI extracts "metformin" from the natural language prompt)
- The `repurposing_suggestions` subworkflow gets the known targets from the conversation context (the AI uses the target results from the first subworkflow)

## Best practices

- **Single responsibility**: Each subworkflow should have one clear purpose
- **Clear parameters**: Use descriptive names and descriptions
- **Pre-set fixed values**: Use `values` for parameters that don't change
- **Test independently**: Verify subworkflows work alone before combining

## Related documentation

- [Workflows](README.md) — the workflow file format a subworkflow is written in.
- [Workflow schema reference](workflow-schema-reference.md#subworkflows) — the full `sub_workflows` field schema and the `indent()` template filter.
- [Subagents](../agent-loop/subagents.md) — the other way a Biorouter run delegates work to a nested agent; read alongside this page when deciding which mechanism fits.
- [Decision models](../agent-loop/decision-models.md#monitors-on-subagents): the monitors a subworkflow's subagent starts with, and who may change them.
- [biorouter CLI command reference](../cli/command-reference.md#run-options) — the `run --workflow` and `--params` flags the examples above use.
