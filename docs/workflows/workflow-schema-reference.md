# Workflow schema reference

> **What this is.** The exhaustive field-by-field specification of the Biorouter workflow file format: every field's type and requirement, the parameter input types, template support, validation rules, and a complete worked example in both YAML and JSON.
> **Status:** Current — this is the authoritative schema document. Where [the workflows index](README.md) summarizes the same fields, this page wins.
> **Audience:** end users authoring workflow files by hand, and anyone automating against the format.

Workflows are reusable Biorouter configurations that package up instructions and settings so the setup can be easily shared and launched by others. This page specifies the file format. If you want the task-oriented walkthrough instead — creating a workflow from a session, running it, sharing it — read [Creating and sharing workflows](creating-and-sharing-workflows.md).

## Contents

- [Workflow file format](#workflow-file-format)
- [Workflow location](#workflow-location)
- [Core workflow schema](#core-workflow-schema)
- [Field specifications](#field-specifications) — [activities](#activities), [extensions](#extensions), [parameters](#parameters), [response](#response), [retry](#retry), [settings](#settings), [subworkflows](#subworkflows)
- [Desktop metadata fields](#desktop-metadata-fields)
- [Template support](#template-support)
- [Validation rules](#validation-rules)
- [Complete workflow example](#complete-workflow-example)
- [Error handling](#error-handling)

## Workflow file format

Workflows can be defined in:

- `.yaml` (recommended) and `.yml` files
- `.json` files

> **Note.** `.yml` files aren't supported by the Biorouter CLI — they work in Desktop only.

See [Creating and sharing workflows](creating-and-sharing-workflows.md) to learn how to create, use, and manage workflows.

## Workflow location

Workflows can be loaded from:

1. Local filesystem:
   - Current directory
   - Directories specified in the [`BIOROUTER_WORKFLOW_PATH`](../configuration/environment-variables.md#workflow-configuration) environment variable

2. GitHub repositories:
   - Configure using the [`BIOROUTER_WORKFLOW_GITHUB_REPO`](../configuration/environment-variables.md#workflow-configuration) configuration key
   - Requires GitHub CLI (`gh`) to be installed and authenticated

[Workflow storage and discovery](workflow-storage-and-discovery.md#workflow-discovery-process) documents the full search order that `biorouter workflow list` uses, including the global and project-local libraries.

## Core workflow schema

Workflows follow this schema structure:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `description` | String | Yes | A detailed description of what the workflow does |
| `instructions` | String | Yes* | Template instructions that can include parameter substitutions |
| `prompt` | String | Yes* | A template prompt that can include parameter substitutions. Required in headless (non-interactive) mode. |
| `title` | String | Yes | A short title describing the workflow |
| [`activities`](#activities) | Array | No | List of example prompts that can include parameter substitutions. Activities appear as clickable bubbles in Biorouter Desktop. |
| [`extensions`](#extensions) | Array | No | List of extension configurations |
| [`capabilities`](#capabilities-turned-on-for-every-chat) | String | No | `explicit` keeps a workflow's chats to its `extensions` list. Without it, the capabilities turned on for every chat stay on beside the list |
| [`parameters`](#parameters) | Array | No | List of parameter definitions for dynamic workflows |
| [`knowledge_bases`](#knowledge-bases) | Object | No | The knowledge bases this workflow works with, and which one it writes to |
| [`response`](#response) | Object | No | Structured output schema for automation workflows |
| [`retry`](#retry) | Object | No | Configuration for automated retry logic with success validation |
| [`settings`](#settings) | Object | No | Configuration for model provider, model name, and other settings |
| [`skills`](#skills) | Array | No | Names of skills whose instructions this workflow requires |
| [`sub_workflows`](#subworkflows) | Array | No | List of subworkflows |
| `version` | String | No | The workflow format version, defaults to "1.0.0" if omitted |

*At least one of `instructions` or `prompt` must be provided.

## Field specifications

### Activities

The `activities` field defines an optional message and clickable activity bubbles (buttons) that appear when a workflow is opened in Biorouter Desktop.

> **Note.** Activities are a Desktop-only feature. When workflows with activities are run via the CLI or as a scheduled job, the `activities` field is ignored and has no effect on workflow execution.

#### Activity types

Activities can be defined in two ways:

1. **Message activity**: Displays the markdown-formatted activity text in an info box above the activity bubbles. For example:

   ```yaml
   activities:
     - "message: **Welcome!** Here's what I can help with:\n\n• 📊 Data analysis\n• 🔍 Code review\n• 📝 Documentation\n\nSelect an option below to begin."
   ```

   Only include one `message:` prefixed activity. Additional `message:` prefixed activities become regular clickable bubbles (and display the literal "message:" text).

2. **Button activities**: Text to display in activity bubbles, which send the activity text as a prompt when clicked

#### Parameter substitution in activities

Activities support [parameter substitution](#parameters), allowing you to create dynamic, personalized activity bubbles. After users provide parameter values in the **Workflow Parameters** dialog, the values are substituted into the activity text before the bubbles are displayed.

#### Example activities configuration

The same workflow is shown twice — first in YAML, then the equivalent JSON.

**YAML:**

```yaml
version: "1.0.0"
title: "Code Review Assistant"
description: "Review code with customizable focus areas"
parameters:
  - key: language
    input_type: string
    requirement: required
    description: "Programming language to review"
  - key: focus
    input_type: string
    requirement: optional
    default: "best practices"
    description: "Review focus area"

activities:
  - "message: Click an option below to start reviewing {{ language }} code with a focus on {{ focus }}."
  - "Review the current file for {{ focus }}"
  - "Suggest improvements for {{ language }} code quality"
  - "Check for security vulnerabilities"
  - "Generate unit tests"
```

**JSON:**

```json
{
  "version": "1.0.0",
  "title": "Code Review Assistant",
  "description": "Review code with customizable focus areas",
  "parameters": [
    {
      "key": "language",
      "input_type": "string",
      "requirement": "required",
      "description": "Programming language to review"
    },
    {
      "key": "focus",
      "input_type": "string",
      "requirement": "optional",
      "default": "best practices",
      "description": "Review focus area"
    }
  ],
  "activities": [
    "message: Click an option below to start reviewing {{ language }} code with a focus on {{ focus }}.",
    "Review the current file for {{ focus }}",
    "Suggest improvements for {{ language }} code quality",
    "Check for security vulnerabilities",
    "Generate unit tests"
  ]
}
```

In this example:
- The message activity displays instructions with substituted parameter values, for example: "Click an option below to start reviewing rust code with a focus on best practices."
- The first two activity bubbles use parameter substitution, for example: "Review the current file for best practices"
- The last two activity bubbles are static prompts that work regardless of parameters

### Extensions

The `extensions` field allows you to specify which Model Context Protocol (MCP) servers and other extensions the workflow needs to function. Each extension in the `extensions` array has the following schema:

#### Extension schema

| Field | Type | Description |
|-------|------|-------------|
| `type` | String | Type of extension (e.g., "stdio") |
| `name` | String | Unique name for the extension |
| `cmd` | String | Command to run the extension |
| `args` | Array | List of arguments for the command |
| `env_keys` | Array | (Optional) Names of environment variables required by the extension |
| `timeout` | Number | Timeout in seconds |
| `bundled` | Boolean | (Optional) Whether the extension is bundled with Biorouter |
| `description` | String | Description of what the extension does |
| `available_tools` | Array | List of tool names within the extension that will be available. When not specified all will be available |

#### Extension types

- **`stdio`**: Standard I/O client with command and arguments
- **`builtin`**: Built-in extension that is part of the bundled Biorouter MCP server
- **`platform`**: Platform extensions that run in the agent process
- **`streamable_http`**: Streamable HTTP client with URI endpoint
- **`frontend`**: Frontend-provided tools called through the frontend
- **`inline_python`**: Inline Python code executed using uvx. Requires `code` field; optional `dependencies` for packages.

#### Example extensions configuration

The same extension list is shown twice — first in YAML, then the equivalent JSON.

**YAML:**

```yaml
extensions:
  - type: stdio
    name: codesearch
    cmd: uvx
    args:
      - mcp_codesearch@latest
    timeout: 300
    bundled: true
    description: "Query your code search service directly from biorouter"
  
  - type: stdio
    name: presidio
    timeout: 300
    cmd: uvx
    args:
      - 'mcp_presidio@latest'
    available_tools:
      - query_logs

  - type: stdio
    name: github-mcp
    cmd: github-mcp-server
    args: []
    env_keys:
      - GITHUB_PERSONAL_ACCESS_TOKEN
    timeout: 60
    description: "GitHub MCP extension for repository operations"
    
  - type: inline_python
    name: data_processor
    code: |
      import pandas as pd
      print("Processing data...")
    dependencies:
      - pandas
      - numpy
    timeout: 120
    description: "Process data using pandas"
```

**JSON:**

```json
{
  "extensions": [
    {
      "type": "stdio",
      "name": "codesearch",
      "cmd": "uvx",
      "args": ["mcp_codesearch@latest"],
      "timeout": 300,
      "bundled": true,
      "description": "Query your code search service directly from biorouter"
    },
    {
      "type": "stdio",
      "name": "presidio",
      "timeout": 300,
      "cmd": "uvx",
      "args": ["mcp_presidio@latest"],
      "available_tools": ["query_logs"]
    },
    {
      "type": "stdio",
      "name": "github-mcp",
      "cmd": "github-mcp-server",
      "args": [],
      "env_keys": ["GITHUB_PERSONAL_ACCESS_TOKEN"],
      "timeout": 60,
      "description": "GitHub MCP extension for repository operations"
    },
    {
      "type": "inline_python",
      "name": "data_processor",
      "code": "import pandas as pd\nprint(\"Processing data...\")",
      "dependencies": ["pandas", "numpy"],
      "timeout": 120,
      "description": "Process data using pandas"
    }
  ]
}
```

#### Capabilities turned on for every chat

A workflow's `extensions` list adds to the capabilities you turned on in **Settings** > **Chat** > **Capabilities**. It does not replace them. A workflow that lists only its own tools still gets Crew, Workspace Control and the other capabilities that are on for every chat on this computer. This holds wherever the workflow runs: a desktop workflow chat, `biorouter run --workflow` and a scheduled run.

- An installed extension, one that runs as its own program or over HTTP, is never added. Only the capabilities are.
- A capability you turned off stays off.
- When the list names a capability itself, its entry is used as written, so an `available_tools` list that narrows it is kept.
- A workflow with no `extensions` key runs with what its caller chose, as before.

To keep a workflow's chats to exactly its list, set `capabilities: explicit`:

```yaml
extensions:
  - type: stdio
    name: github-mcp
    cmd: github-mcp-server
    args: []
capabilities: explicit
```

`explicit` is the only value, and any other spelling fails to load the workflow. It has no effect on a workflow without an `extensions` key. A workflow that declares [knowledge bases](#knowledge-bases) still gets Knowledge. Its chats are told which capabilities are on for other chats but off here, by name, such as "Crew, Workspace Control", so the model can say so when you ask for one and point you to a new chat or the workflow's `extensions`.

A chat that another chat opens with Workspace Control and its own list of extensions keeps the capabilities that are on for every chat in the same way.

#### Extension secrets

This feature is only available through the CLI.

If a workflow uses an extension that requires a secret, Biorouter can prompt users to provide the secret when running the workflow:

1. When a workflow is loaded, Biorouter scans all extensions (including those in subworkflows) for `env_keys` fields
2. If any required environment variables are missing from the secure keyring, Biorouter prompts the user to enter them
3. Values are stored securely in the system keyring and reused for subsequent runs

To update a stored secret, remove it from the system keyring and run the workflow again to be re-prompted.

> **Note.** This feature is designed to prompt for and securely store secrets (such as API keys), but `env_keys` can include any environment variable needed by the extension (such as API endpoints, configuration values, etc.).

Users can press `ESC` to skip entering a variable if it's optional for the extension.

### Parameters

The `parameters` field allows you to create dynamic, reusable workflows that can be customized for different contexts. Parameters define placeholders that users fill in when running the workflow, making the workflow more flexible and adaptable.

Parameter substitution uses Jinja-style template syntax with `{{ parameter_name }}` placeholders. Each parameter in the `parameters` array has the following schema:

#### Parameter schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | String | Yes | Unique identifier for the parameter |
| `input_type` | String | Yes | Type of input: `"string"` (default), `"number"`, `"boolean"`, `"date"`, `"file"`, or `"select"` |
| `requirement` | String | Yes | One of: "required", "optional", or "user_prompt" |
| `description` | String | Yes | Human-readable description of the parameter |
| `default` | String | No | Default value for optional parameters |
| `options` | Array | No | List of available choices (required for `select` input type) |

#### Parameter requirements

- `required`: Parameter must be provided when using the workflow
- `optional`: Can be omitted if a default value is specified
- `user_prompt`: Will interactively prompt the user for input if not provided

The `required` and `optional` parameters work best for workflows opened in Biorouter Desktop. If a value isn't provided for a `user_prompt` parameter, the parameter won't be substituted and may appear as literal `{{ parameter_name }}` text in the workflow output.

#### Input types

- `string`: Default type. The parameter value is used as-is in template substitution
- `number`: Numeric values. Desktop UI provides number input validation
- `boolean`: True/false values. Desktop UI shows dropdown with "True"/"False" options
- `date`: Date values. Currently renders as text input
- `file`: The parameter value should be a file path. Biorouter reads the file contents and substitutes the actual content (not the path) into the template
- `select`: Dropdown selection with predefined options. Requires `options` field

**Example:**
```yaml
parameters:
  - key: max_files
    input_type: number
    requirement: optional
    default: "10"
    description: "Maximum files to process"
  
  - key: output_format
    input_type: select
    requirement: required
    description: "Choose output format"
    options:
      - json
      - markdown
      - csv
  
  - key: enable_debug
    input_type: boolean
    requirement: optional
    default: "false"
    description: "Enable debug mode"
  
  - key: source_code
    input_type: file
    requirement: required
    description: "Path to the source code file to analyze"

prompt: "Process {{ max_files }} files in {{ output_format }} format. Debug: {{ enable_debug }}. Code:\n\n{{ source_code }}"
```

> **Warning.** Parameter rules enforced at load time:
> - Optional parameters MUST have a default value specified
> - Required parameters cannot have default values
> - File parameters cannot have default values regardless of requirement type, to prevent unintended importing of sensitive files
> - Select parameters MUST have an `options` field with available choices
> - Parameter keys must match any template variables used in instructions, prompt, or activities

#### Parameter substitution in Desktop

When a workflow with parameters is opened in Biorouter Desktop, users are presented with a **Workflow Parameters** dialog where they can:
- Provide values for required parameters
- Modify or accept default values for optional parameters
- Enter values for `user_prompt` parameters

Once parameter values are submitted, they are substituted into the workflow's `instructions`, `prompt`, and `activities` fields before the workflow starts.

### Response

The `response` field enables workflows to enforce a final structured JSON output. When you specify a `json_schema`, Biorouter will:

1. **Validate the output**: Validates the output JSON against your JSON schema with basic JSON schema validations
2. **Final structured output**: Ensure the final output of the agent is a response matching your JSON structure

This feature is designed for **non-interactive automation** to ensure consistent, parseable output. Workflows can produce structured output when run from either the Biorouter CLI or Biorouter Desktop. See [use cases and ideas for automation workflows](creating-and-sharing-workflows.md#structured-output-for-automation).

#### Response schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `json_schema` | Object | Yes | [JSON schema](https://json-schema.org/) for output validation |

#### Basic response structure

```yaml
response:
  json_schema:
    type: object
    properties:
      # Define your fields here, with their type and description
    required:
      # List required field names
```

#### Simple response example

```yaml
version: "1.0.0"
title: "Task Summary"
description: "Summarize completed tasks"
prompt: "Summarize the tasks you completed"
response:
  json_schema:
    type: object
    properties:
      summary:
        type: string
        description: "Brief summary of work done"
      tasks_completed:
        type: number
        description: "Number of tasks finished"
      next_steps:
        type: array
        items:
          type: string
        description: "Recommended next actions"
    required:
      - summary
      - tasks_completed
```

### Retry

The `retry` field enables workflows to automatically retry execution if success criteria are not met. This is useful for workflows that might need multiple attempts to achieve their goal, or for implementing automated validation and recovery workflows.

#### Retry schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `max_retries` | Number | Yes | Maximum number of retry attempts |
| `checks` | Array | Yes | List of success check configurations |
| `timeout_seconds` | Number | No | Timeout for success check commands (default: 300 seconds) |
| `on_failure_timeout_seconds` | Number | No | Timeout for on_failure commands (default: 600 seconds) |
| `on_failure` | String | No | Shell command to run when a retry attempt fails |

#### Success check configuration

Each success check in the `checks` array has the following schema:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | String | Yes | `shell`, `file_exists`, `output_contains` or `json_schema` |

The fields each type takes:

| `type` | Fields | Passes when |
|--------|--------|-------------|
| `shell` | `command` | The command exits 0 |
| `file_exists` | `path` | A file or directory exists at `path`. A relative path resolves against the session working directory for the interactive gate, and the process working directory for a workflow retry |
| `output_contains` | `command`, `substring` | The command's combined stdout+stderr contains `substring`. The exit status is **ignored** — this checks the output's content (`0 failed`, `PASS`, a coverage figure) |
| `json_schema` | `path`, `schema` | The JSON file at `path` parses and validates against the inline JSON Schema in `schema` |

#### How retry logic works

1. **Workflow execution**: The workflow runs normally with the provided instructions
2. **Success validation**: After completion, all success checks are executed in order
3. **Retry decision**: If any success check fails and retry attempts remain:
   - Execute the on_failure command (if configured)
   - Reset the agent's message history to initial state
   - Increment retry counter and restart execution
4. **Completion**: Process stops when either:
   - All success checks pass (success)
   - Maximum retry attempts are reached (failure)

#### Basic retry example

```yaml
version: "1.0.0"
title: "Counter Increment Task"
description: "Increment a counter until it reaches target value"
prompt: "Increment the counter value in /tmp/counter.txt by 1."

retry:
  max_retries: 5
  timeout_seconds: 10
  checks:
    - type: shell
      command: "test $(cat /tmp/counter.txt 2>/dev/null || echo 0) -ge 3"
  on_failure: "echo 'Counter is at:' $(cat /tmp/counter.txt 2>/dev/null || echo 0) '(need 3 to succeed)'"
```

#### Advanced retry example

```yaml
version: "1.0.0"
title: "Service Health Check"
description: "Start service and verify it's running properly"
prompt: "Start the web service and verify it responds to health checks"

retry:
  max_retries: 3
  timeout_seconds: 30
  on_failure_timeout_seconds: 60
  checks:
    - type: shell
      command: "curl -f http://localhost:8080/health"
    - type: shell  
      command: "pgrep -f 'web-service' > /dev/null"
  on_failure: "systemctl stop web-service || killall web-service"
```

#### Retry environment variables

You can configure retry behavior globally using environment variables:

- `BIOROUTER_WORKFLOW_RETRY_TIMEOUT_SECONDS`: Global timeout for success check commands
- `BIOROUTER_WORKFLOW_ON_FAILURE_TIMEOUT_SECONDS`: Global timeout for on_failure commands

These environment variables are overridden by workflow-specific timeout configurations.

### Settings

The `settings` field allows you to configure the AI model and provider settings for the workflow. This overrides the default configuration when the workflow is executed.

#### Settings schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `biorouter_provider` | String | No | The AI provider to use (e.g., "anthropic", "openai") |
| `biorouter_model` | String | No | The specific model name to use |
| `temperature` | Number | No | The temperature setting for the model (typically 0.0-1.0) |

#### Example settings configuration

```yaml
settings:
  biorouter_provider: "anthropic"
  biorouter_model: "claude-sonnet-4-20250514"
  temperature: 0.7
```

```yaml
settings:
  biorouter_provider: "openai"
  biorouter_model: "gpt-4o"
  temperature: 0.3
```

> **Note.** Settings specified in a workflow will override your default Biorouter configuration when that workflow is executed. If no settings are specified, Biorouter will use your configured defaults.

### Skills

The `skills` field names the [skills](../extensions/skill-catalog.md) whose instructions this workflow requires. Their bodies are inlined into the system prompt before the first model call, so a workflow can depend on a documented procedure without restating it.

This is stricter than the model's own skill search on purpose: a workflow *requires* a procedure, so naming one that is not installed, or one the conversation has switched off, stops the run with an error rather than degrading to a hint the model may ignore.

An entry may name either a single skill or a whole **bundle** — the group an installed skill package registers. A bundle expands to every skill in it, and naming a bundle together with one of its members inlines that member once.

```yaml
skills:
  - spoke-knowledge-graph   # one skill
  - office-pack             # every skill in the bundle
```

### Knowledge bases

The `knowledge_bases` field states which [knowledge bases](../knowledge-base/README.md) a session started from this workflow can see, and which one it writes to.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `visible` | Array | No | The base ids the session may search. `default` is added to this set automatically |
| `default` | String | No | The primary base: the write target, and the single-base read target |

Omitting the whole `knowledge_bases` key and giving an empty one mean different things. Absent means "this workflow has nothing to say about knowledge bases", so the session keeps whatever selection it would otherwise have. An empty `visible` with no `default` is a statement — "this workflow sees no knowledge bases" — and is applied.

```yaml
knowledge_bases:
  default: research-kb
  visible:
    - research-kb
    - protocols
```

> **Note.** A workflow that declares this key needs the Knowledge capability to honour it, so Biorouter loads it for the session even when the workflow's `extensions` list omits it.

### Subworkflows

The `sub_workflows` field specifies the [subworkflows](subworkflows.md) that the main workflow calls to perform specific tasks. Each subworkflow in the `sub_workflows` array has the following schema:

#### Subworkflow schema

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | String | Yes | Unique identifier for the subworkflow |
| `path` | String | Yes | Relative or absolute path to the subworkflow file |
| `values` | Object | No | Pre-configured parameter values that are passed to the subworkflow |
| `sequential_when_repeated` | Boolean | No | Forces sequential execution of multiple subworkflow instances. Set it to `false` to allow multiple instances to run in parallel |
| `description` | String | No | Optional description of the subworkflow |

#### Example subworkflow configuration

```yaml
sub_workflows:
  - name: "security_scan"
    path: "./subworkflows/security-analysis.yaml"
    values:  # in key-value format: {parameter_name}: {parameter_value}
      scan_level: "comprehensive"
      include_dependencies: "true"
  
  - name: "quality_check"
    path: "./subworkflows/quality-analysis.yaml"
    description: "Performs code quality analysis"
```

## Desktop metadata fields

Workflows saved from Biorouter Desktop include additional metadata fields. These fields are used by the Desktop app for organization and management but are ignored by CLI operations.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `workflow` | Object | Yes | Contains all workflow fields (`title`, `description`, `instructions`, etc.) |
| `name` | String | Yes | Display name used in Workflow Library |
| `isGlobal` | Boolean | Yes | Whether the workflow is available globally or locally to a project |
| `lastModified` | String | Yes | ISO timestamp of when the workflow was last modified |
| `isArchived` | Boolean | Yes | Whether the workflow is archived in the Desktop interface |

### CLI format

The plain format the CLI reads and writes — workflow fields at the top level.

**YAML:**

```yaml
version: "1.0.0"
title: "Code Review Assistant"
description: "Automated code review with best practices"
instructions: "You are a code reviewer..."
prompt: "Review the code in this repository"
extensions: []
```

**JSON:**

```json
{
  "version": "1.0.0",
  "title": "Code Review Assistant",
  "description": "Automated code review with best practices",
  "instructions": "You are a code reviewer...",
  "prompt": "Review the code in this repository",
  "extensions": []
}
```

### Desktop format

The same workflow as saved by Desktop — the CLI fields nested under `workflow`, wrapped in the metadata fields above.

**YAML:**

```yaml
name: "Code Review Assistant"
workflow:
  version: "1.0.0"
  title: "Code Review Assistant"
  description: "Automated code review with best practices"
  instructions: "You are a code reviewer..."
  prompt: "Review the code in this repository"
  extensions: []
isGlobal: true
lastModified: 2025-07-02T03:46:46.778Z
isArchived: false
```

**JSON:**

```json
{
  "name": "Code Review Assistant",
  "workflow": {
    "version": "1.0.0",
    "title": "Code Review Assistant",
    "description": "Automated code review with best practices",
    "instructions": "You are a code reviewer...",
    "prompt": "Review the code in this repository",
    "extensions": []
  },
  "isGlobal": true,
  "lastModified": "2025-07-02T03:46:46.778Z",
  "isArchived": false
}
```

## Template support

Workflows support Jinja-style template syntax in `instructions`, `prompt`, and `activities` fields for parameter substitution:

```yaml
instructions: "Follow these steps with {{ parameter_name }}"
prompt: "Your task is to {{ action }}"
activities:
  - "Process {{ parameter_name }} with {{ action }}"
```

Advanced template features include:
- [Template inheritance](#template-inheritance) using `{% extends "parent.yaml" %}`
- Blocks that can be defined and overridden:
  ```yaml
  {% block content %}
  Default content
  {% endblock %}
  ```
- The [`indent()` template filter](#indent-filter-for-multi-line-values)

### Template inheritance

Use `{% extends "parent.yaml" %}` for template inheritance:

**Parent workflow (`parent.yaml`):**
```yaml
version: "1.0.0"
title: "Parent Workflow"
description: "Base workflow template"
prompt: |
  {% block prompt %}
  Default prompt text
  {% endblock %}
```

**Child workflow:**
```yaml
{% extends "parent.yaml" %}
{% block prompt %}
Modified prompt text
{% endblock %}
```

### indent() filter for multi-line values

Use the `indent()` filter to ensure multi-line parameter values are properly indented and can be resolved as valid JSON or YAML format. This example uses `{{ raw_data | indent(2) }}` to specify an indentation of two spaces when passing data to a subworkflow:

```yaml
sub_workflows:
  - name: "analyze"
    path: "./analyze.yaml"
    values:
      content: |
        {{ raw_data | indent(2) }}
```

### Built-in parameters

Built-in template parameters are automatically supported and don't need to be defined in the `parameters` array.

| Parameter | Description |
|-----------|-------------|
| `workflow_dir` | Automatically set to the directory containing the workflow file. Use it to reference companion files, for example: `{{ workflow_dir }}/style-guide.md` |

## Validation rules

Validation rules from `crates/biorouter/src/workflow/validate_workflow.rs` ([source on GitHub](https://github.com/BaranziniLab/biorouter/blob/main/crates/biorouter/src/workflow/validate_workflow.rs)) are enforced when loading workflows and used by the [`biorouter workflow validate`](../cli/command-reference.md#workflow) subcommand:

### Workflow-level validation

- `validate_prompt_or_instructions` - At least one of `instructions` or `prompt` must be present
- `validate_json_schema` - JSON response schema must be valid if `response.json_schema` is specified

### Parameter validation

- `validate_parameters_in_template` - All template variables must have corresponding parameter definitions, and all defined parameters must be used (no unused parameters)
- `validate_optional_parameters` - Optional parameters must have default values
- `validate_optional_parameters` - File parameters cannot have default values to prevent importing sensitive files

> **Note.** Basic field requirements (required fields, types, character limits) are documented in the [core workflow schema](#core-workflow-schema) table.

## Complete workflow example

One workflow exercising most of the schema at once, shown in YAML and then the equivalent JSON. Reading top to bottom, it demonstrates:

- All four non-default parameter kinds — `string` (`required_param`), `number` with a default (`file_count`), `select` with `options` (`output_format`), and `file` (`config_file`)
- Template substitution of those parameters in `instructions`
- A `stdio` extension declaration
- `settings` overriding the provider, model, and temperature
- `retry` with one shell check and an `on_failure` cleanup command
- `response.json_schema` enforcing a structured final output

**YAML:**

```yaml
version: "1.0.0"
title: "Example Workflow"
description: "A sample workflow demonstrating the format"
instructions: "Process {{ file_count }} files using {{ required_param }} and output in {{ output_format }} format. Configuration: {{ config_file }}"
prompt: "Start processing with the provided parameters"
parameters:
  - key: required_param
    input_type: string
    requirement: required
    description: "A required text parameter"
  
  - key: file_count
    input_type: number
    requirement: optional
    default: 10
    description: "Maximum number of files to process"
  
  - key: output_format
    input_type: select
    requirement: required
    description: "Choose the output format"
    options:
      - json
      - markdown
      - csv
  
  - key: config_file
    input_type: file
    requirement: required
    description: "Path to configuration file"

extensions:
  - type: stdio
    name: codesearch
    cmd: uvx
    args:
      - mcp_codesearch@latest
    timeout: 300
    bundled: true
    description: "Query codesearch directly from biorouter"

settings:
  biorouter_provider: "anthropic"
  biorouter_model: "claude-sonnet-4-20250514"
  temperature: 0.7

retry:
  max_retries: 3
  timeout_seconds: 30
  checks:
    - type: shell
      command: "echo 'Task validation check passed'"
  on_failure: "echo 'Retry attempt failed, cleaning up...'"

response:
  json_schema:
    type: object
    properties:
      result:
        type: string
        description: "The main result of the task"
      details:
        type: array
        items:
          type: string
        description: "Additional details of steps taken"
    required:
      - result
      - details
```

**JSON:**

```json
{
  "version": "1.0.0",
  "title": "Example Workflow",
  "description": "A sample workflow demonstrating the format",
  "instructions": "Process {{ file_count }} files using {{ required_param }} and output in {{ output_format }} format. Configuration: {{ config_file }}",
  "prompt": "Start processing with the provided parameters",
  "parameters": [
    {
      "key": "required_param",
      "input_type": "string",
      "requirement": "required",
      "description": "A required text parameter"
    },
    {
      "key": "file_count",
      "input_type": "number",
      "requirement": "optional",
      "default": "10",
      "description": "Maximum number of files to process"
    },
    {
      "key": "output_format",
      "input_type": "select",
      "requirement": "required",
      "description": "Choose the output format",
      "options": ["json", "markdown", "csv"]
    },
    {
      "key": "config_file",
      "input_type": "file",
      "requirement": "required",
      "description": "Path to configuration file"
    }
  ],
  "extensions": [
    {
      "type": "stdio",
      "name": "codesearch",
      "cmd": "uvx",
      "args": ["mcp_codesearch@latest"],
      "timeout": 300,
      "bundled": true,
      "description": "Query codesearch directly from biorouter"
    }
  ],
  "settings": {
    "biorouter_provider": "anthropic",
    "biorouter_model": "claude-sonnet-4-20250514",
    "temperature": 0.7
  },
  "retry": {
    "max_retries": 3,
    "timeout_seconds": 30,
    "checks": [
      {
        "type": "shell",
        "command": "echo 'Task validation check passed'"
      }
    ],
    "on_failure": "echo 'Retry attempt failed, cleaning up...'"
  },
  "response": {
    "json_schema": {
      "type": "object",
      "properties": {
        "result": {
          "type": "string",
          "description": "The main result of the task"
        },
        "details": {
          "type": "array",
          "items": {
            "type": "string"
          },
          "description": "Additional details of steps taken"
        }
      },
      "required": ["result", "details"]
    }
  }
}
```

## Error handling

Common errors to watch for:

- Missing required parameters
- Optional parameters without default values
- Template variables without parameter definitions
- Invalid YAML/JSON syntax
- Missing required fields
- Invalid extension configurations
- Invalid retry configuration (missing required fields, invalid shell commands)

When these occur, Biorouter will provide helpful error messages indicating what needs to be fixed.

### Retry-specific errors

- **Invalid success checks**: Shell commands that cannot be executed or have syntax errors
- **Timeout errors**: Success checks or on_failure commands that exceed their timeout limits
- **Max retries exceeded**: When all retry attempts are exhausted without success
- **Missing required retry fields**: When `max_retries` or `checks` are not specified

## Related documentation

- [Workflows](README.md) — the short orientation to this format, plus an index of the rest of this folder.
- [Creating and sharing workflows](creating-and-sharing-workflows.md) — the task walkthrough for producing, running, and sharing files that follow this schema.
- [Subworkflows](subworkflows.md) — worked examples of the `sub_workflows` field specified above.
- [Workflow storage and discovery](workflow-storage-and-discovery.md) — where these files live and how Biorouter finds them.
- [biorouter CLI command reference](../cli/command-reference.md#workflow) — `biorouter workflow validate`, which enforces the validation rules above.
