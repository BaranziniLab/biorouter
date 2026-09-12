# biorouter CLI command reference

> **What this is.** The reference for every `biorouter` command-line subcommand and its flags, plus the slash commands, themes, and keyboard shortcuts available inside an interactive session.
> **Status:** Current.
> **Audience:** end users

biorouter ships a command-line interface (CLI) for managing sessions, configuration, and extensions, and for running tasks headlessly. This page lists the commands you can run from your shell, then the controls available once you are inside an interactive session. For the manual verification script that exercises this same surface, see the [CLI QA checklist](qa-checklist.md).

## Contents

- [Flag naming conventions](#flag-naming-conventions)
- [Core commands](#core-commands)
- [Session management](#session-management)
- [Task execution](#task-execution)
- [Project management](#project-management)
- [Interface](#interface)
- [Terminal integration](#terminal-integration)
- [Interactive session features](#interactive-session-features)
- [Navigation and controls](#navigation-and-controls)

## Flag naming conventions

The biorouter CLI follows consistent patterns for flag naming to make commands intuitive and predictable:

- **`--session-id`**: Used for session identifiers (e.g., `20251108_1`)
- **`--schedule-id`**: Used for schedule job identifiers (e.g., `daily-report`)
- **`-n, --name`**: Used for human-readable names
- **`--path`**: Used for file paths (legacy support)
- **`-o, --output`**: Used for output file paths
- **`-r, --resume` or `-r, --regex`**: Context-dependent (resume for sessions, regex for filters)
- **`-v, --verbose`**: Used for verbose output
- **`-l, --limit`**: Used for limiting result counts
- **`-f, --format`**: Used for specifying output formats
- **`-w, --working_dir`**: Used for working directory filters

## Core commands

### help

Display the help menu.

**Usage:**

```bash
biorouter --help
```

### configure

Configure biorouter settings - providers, extensions, etc.

**Usage:**

```bash
biorouter configure
```

`configure` is interactive and needs a terminal. Run from a script, a pipe or
anywhere else without one, it changes nothing and exits with status `2`, naming
the non-interactive route instead: choose the model with
`biorouter models set --provider <provider> --model <model>` and pass the
provider's API key in its environment variable (for example `OPENAI_API_KEY`).

### info [options]

Shows biorouter information, including the version, configuration file location, session storage, and logs.

**Options:**

- **`-v, --verbose`**: Show detailed configuration settings, including environment variables and enabled extensions

**Usage:**

```bash
biorouter info
```

### models

Inspect and update model/provider configuration from the CLI.

**Usage:**

```bash
# Show the configured provider and model
biorouter models current

# List available providers
biorouter models providers

# List known models for a provider
biorouter models list openai
biorouter models list ollama --format json

# Set the default provider and model
biorouter models set --provider openai --model gpt-5.5
```

### version

Check the current biorouter version you have installed.

**Usage:**

```bash
biorouter --version
```

## Session management

> **Note.** biorouter stores sessions in a SQLite database (`sessions.db`) rather than individual `.jsonl` files, a change introduced in version 1.10.0. Sessions that predate the change are automatically imported into the database. Legacy `.jsonl` files remain on disk but are no longer managed by biorouter.

### session [options]

Start or resume interactive chat sessions.

**Basic Options:**

- **`--session-id <session_id>`**: Specify a session by its ID (e.g., '20251108_1')
- **`-n, --name <name>`**: Give the session a name
- **`--path <path>`**: Legacy parameter for specifying session by file path
- **`-r, --resume`**: Resume a previous session
- **`--history`**: Show previous messages when resuming a session
- **`--debug`**: Enable debug mode to output complete tool responses, detailed parameter values, and full file paths
- **`--max-tool-repetitions <NUMBER>`**: Set the maximum number of times the same tool can be called consecutively with identical parameters. Helps prevent infinite loops.
- **`--max-turns <NUMBER>`**: Set the maximum number of turns allowed without user input (default: 1000)

**Extension Options:**

- **`--with-extension <command>`**: Add stdio extensions
- **`--with-streamable-http-extension <url>`**: Add remote extensions over Streamable HTTP
- **`--with-builtin <id>`**: Enable built-in extensions (e.g., 'developer', 'computercontroller')

**Usage:**

```bash
# Start a basic session
biorouter session -n my-project

# Resume a previous session
biorouter session --resume -n my-project
biorouter session --resume --session-id 20251108_2
biorouter session --resume --path ./session.json    # exported session
biorouter session --resume --path ./session.jsonl   # legacy session storage

# Start with extensions
biorouter session --with-extension "npx -y @modelcontextprotocol/server-memory"
biorouter session --with-builtin developer
biorouter session --with-streamable-http-extension "http://localhost:8080/mcp"

# Advanced: Mix multiple extension types
biorouter session \
  --with-extension "echo hello" \
  --with-streamable-http-extension "http://localhost:8080/mcp" \
  --with-builtin "developer"

# Control session behavior
biorouter session -n my-session --debug --max-turns 25
```

### session list [options]

List all saved sessions.

**Options:**

- **`-f, --format <format>`**: Specify output format (`text` or `json`). Default is `text`
- **`--ascending`**: Sort sessions by date in ascending order (oldest first)
- **`-w, --working_dir <path>`**: Filter sessions by working directory
- **`-l, --limit <number>`**: Limit the number of results
- **`--subagents`**: Include subagent runs, nested under the session that spawned them (and sessions that have no messages)
- **`--include-empty`**: Include sessions that have not recorded a message yet — what a `biorouter` run that exited before its first prompt, `doctor --fix` and `term init` leave behind. They are hidden by default, as they are in the desktop app's History

**Usage:**

```bash
# List all sessions in text format (default)
biorouter session list

# List sessions in JSON format
biorouter session list --format json

# Sort sessions by date in ascending order
biorouter session list --ascending

# Filter sessions by working directory
biorouter session list -w ~/projects/myapp

# List only the 10 most recent sessions
biorouter session list --limit 10

# Show subagent runs too, grouped under the session that spawned them
biorouter session list --subagents
```

#### Listing subagent runs

By default a listing shows only your own chats and scheduled runs — the work an
agent delegates to a subagent is filtered out, so a fan-out of five children is
invisible from the terminal. `--subagents` includes those runs and nests each one
under the session that spawned it:

```
20260731_1 - Migration review - 2026-07-31 15:04:11 UTC
  └─ Subagent: audit the migration for data loss [● live]  20260731_2  14 msgs  2026-07-31 15:09
  └─ Subagent: benchmark the covering index [○ done]  20260731_3  8 msgs  2026-07-31 15:07
```

Each child line carries its own label, its session id and its message count —
enough to tell two siblings of one fan-out apart, and enough to hand the id to
`biorouter session --resume` or `biorouter session watch`.

Two behaviours differ from a plain listing:

- **`--limit` counts top-level sessions, not rows.** `--limit 5` returns five
  parents with all of their children, rather than five rows total. Truncating
  before grouping would let one parent's six children consume the whole budget.
- **Live/finished state is read from a running daemon**, which is the only
  authority on it (`biorouterd` tracks in-flight turns in memory). When no daemon
  can be reached — none running, or `BIOROUTER_SERVER__SECRET_KEY` not set or not
  matching — every run reads `· state unknown` and the reason is printed once on
  stderr. It never reads `done`, because "finished" and "not knowable from here"
  are different answers and only one of them is safe to guess.

`--name` resolves subagent runs too, so a run found this way can be addressed by
its label as well as by its id.

### session remove [options]

Remove one or more saved sessions.

**Options:**

- **`--session-id <session_id>`**: Remove a specific session by its session ID — any session, including a subagent run or a session with no messages
- **`-n, --name <name>`**: Remove the one session carrying this name. When several sessions share it (every session a bare `biorouter` creates is listed as `New chat` until it is renamed), nothing is removed and the matching IDs are listed
- **`-r, --regex <pattern>`**: Remove every session whose ID matches a regex pattern, among the sessions `session list` shows
- **`--include-empty`**: With `--regex` or the picker, also match sessions that have not recorded a message yet
- **`--subagents`**: With `--regex` or the picker, also match subagent runs (and sessions with no messages)
- **`-y, --yes`**: Remove without asking for confirmation. Required when the command is not run from a terminal
- **`--path <path>`**: Remove a specific session by its file path (legacy)

**Usage:**

```bash
# Interactive removal (prompts you to choose sessions)
biorouter session remove

# Remove a specific session by ID
biorouter session remove --session-id 20251108_3

# The same, from a script: no confirmation prompt
biorouter session remove --session-id 20251108_3 --yes

# Remove a specific session by name
biorouter session remove -n my-project

# Remove all sessions starting with "project-"
biorouter session remove -r "project-.*"

# Remove all sessions containing "migration"
biorouter session remove -r ".*migration.*"
```

> **Warning.** Session removal is permanent and cannot be undone. Unless you pass `--yes`, biorouter shows which sessions will be removed and asks for confirmation before deleting. Asking needs a terminal: without one — in a script or a pipe, even with `y` piped in — the command removes nothing and exits with status `2`, asking for `--yes`.

Removing a session also removes its per-turn usage records — which model and provider answered each reply, when, and how many tokens it used — whether you remove it here or delete it from the desktop app's chat history. The tokens it spent are first added to an anonymous total, kept per day, model and provider with nothing that identifies the chat, so `biorouter usage` and the desktop app's Usage panel still match your provider's own billing meter. The Home heatmap and its token tiles count only the chats that still exist.

### session export [options]

Export sessions in different formats for backup, sharing, migration, or documentation purposes.

**Options:**

- **`--session-id <session_id>`**: Export a specific session by ID
- **`-n, --name <name>`**: Export a specific session by name
- **`--path <path>`**: Export a specific session by file path (legacy)
- **`-o, --output <file>`**: Save exported content to a file (default: stdout)
- **`--format <format>`**: Output format: `markdown`, `json`, `yaml`. Default is `markdown`

**Export Formats:**

- **`json`**: Complete session backup preserving all data including conversation history, metadata, and settings
- **`yaml`**: Complete session backup in YAML format
- **`markdown`**: Default format that creates a formatted, readable version of the conversation for documentation and sharing

**Usage:**

```bash
# Interactive export
biorouter session export

# Export specific session as JSON for backup
biorouter session export -n my-session --format json -o session-backup.json

# Export specific session as readable markdown
biorouter session export -n my-session -o session.md

# Export to stdout in different formats
biorouter session export --session-id 20251108_4 --format json
biorouter session export -n my-session --format yaml

# Export session by path (legacy)
biorouter session export --path ./my-session.jsonl -o exported.md
```

### Live-session commands and the daemon

`session watch`, `session send`, `session attach`, and `session cancel` do not open an agent in your terminal. They talk to a running `biorouterd` over HTTP, and the daemon stays the only writer to the conversation — which is what makes it safe to join a session another process is already running.

Each of them needs two things:

- **A daemon listening on `127.0.0.1`.** The port comes from `BIOROUTER_PORT`, defaulting to `3000`. If nothing answers, the command stops with `no Biorouter daemon is listening on 127.0.0.1:<port>`.
- **`BIOROUTER_SERVER__SECRET_KEY` set in your shell, matching the key the daemon was started with.** `biorouterd` generates a random key when the variable is unset, and no client can then authenticate. Start the daemon with a key you choose and reuse it:

```bash
BIOROUTER_SERVER__SECRET_KEY=<key> biorouterd agent
BIOROUTER_SERVER__SECRET_KEY=<key> biorouter session watch <session_id>
```

A mismatched key surfaces as `HTTP 401` with that hint attached.

### session watch [options]

Stream a session's live events into your terminal — the same frames biorouter Desktop renders, printed as lines. Watching is read-only: it never writes to the conversation, and stopping the watch never stops the session.

**Requires a running daemon.** See [Live-session commands and the daemon](#live-session-commands-and-the-daemon).

**Arguments:**

- **`<SESSION_ID>`** (required): The session to observe

**Options:**

- **`--follow`**: Keep watching after the current turn ends. Without it, `watch` exits as soon as the session reports a finish or an error

**Usage:**

```bash
# Watch the running turn and exit when it ends
biorouter session watch 20251108_2

# Keep watching across turns until you press Ctrl+C
biorouter session watch 20251108_2 --follow
```

### session send [options]

Send a prompt into an existing session and stream the resulting turn, without opening an interactive chat. This is the scriptable equivalent of typing one message into a session.

**Requires a running daemon.** See [Live-session commands and the daemon](#live-session-commands-and-the-daemon).

**Arguments:**

- **`<SESSION_ID>`** (required): The session to send to
- **`<TEXT>`** (required): The prompt text

**Options:**

- **`--no-wait`**: Return as soon as the daemon accepts the turn, printing `[started] turn <turn_id> in session <session_id>`, instead of streaming it to completion

**Usage:**

```bash
# Send a prompt and watch the turn to completion
biorouter session send 20251108_2 "summarize what you have found so far"

# Kick off a turn and return immediately
biorouter session send 20251108_2 "run the full test suite" --no-wait
```

A turn started with `--no-wait` runs on in the daemon: `biorouter session watch
<session_id>` follows it and `biorouter session cancel <session_id>` stops it.
The daemon stops a turn once nothing has been attached to its reply stream for
five minutes, and `session watch` does not count as attached — so `--no-wait`
suits turns shorter than that. Leave `--no-wait` off for a longer one.

### session attach [options]

Join a session that is running *right now*. `attach` prints the conversation so far, then follows it live, and anything you type is delivered into the running turn (or starts a new one if the session is idle).

**Requires a running daemon.** See [Live-session commands and the daemon](#live-session-commands-and-the-daemon).

**Arguments:**

- **`<SESSION_ID>`** (optional): The session to attach to

**Options:**

- **`--name <NAME>`**: Attach by session name instead of ID. Refuses, and lists the candidates, if several sessions share that name
- **`--of <PARENT_ID>`**: Attach to the running subagent of this parent session. Errors, listing what it found, if that parent has no subagent with a turn in flight or has more than one
- **`--read-only`**: Observe only — do not read stdin and do not send anything

Give **exactly one** target: a session ID, `--name`, or `--of`. Passing none, or more than one, is an error. `biorouter session list --subagents` lists the sessions and subagent runs you can address.

`Ctrl+C` detaches and leaves the session running. If a turn *you* started from here is still streaming, the first `Ctrl+C` warns that leaving would cancel it; press it again to leave anyway.

**Usage:**

```bash
# Join a running session by ID
biorouter session attach 20251108_2

# Join by name
biorouter session attach --name migration-review

# Join the running subagent of a parent session
biorouter session attach --of 20251108_2

# Follow a run without being able to steer it
biorouter session attach 20251108_2 --read-only
```

> **Note.** Use `attach`, not `session --resume`, on a session that is running. Resuming builds a second agent over the same conversation inside your CLI process, and that agent does not share the daemon's turn lock — leaving two uncoordinated writers on one session. Reserve `--resume` for finished transcripts.

### session cancel

Stop the turn a session is currently running. This is the same action as the Stop button in biorouter Desktop.

**Requires a running daemon.** See [Live-session commands and the daemon](#live-session-commands-and-the-daemon).

**Arguments:**

- **`<SESSION_ID>`** (required): The session whose running turn should be stopped

Cancelling is idempotent: a session with no turn in flight is not an error, it reports `nothing to cancel: this session had no turn in flight`.

**Usage:**

```bash
# Stop the turn a session is running
biorouter session cancel 20251108_2
```

### session diagnostics [options]

Generate a comprehensive diagnostics bundle for troubleshooting issues with a specific session.

**Options:**

- **`--session-id <session_id>`**: Generate diagnostics for a specific session by ID
- **`-n, --name <name>`**: Generate diagnostics for a specific session by name
- **`--path <path>`**: Generate diagnostics for a specific session by file path (legacy)
- **`-o, --output <file>`**: Save diagnostics bundle to a specific file path (default: `diagnostics_{session_id}.zip`)

**What's included:**

- **System Information**: App version, operating system, architecture, and timestamp
- **Session Data**: Complete conversation messages and history for the specified session
- **Configuration Files**: Your [configuration files](../configuration/config-file-reference.md) (if they exist)
- **Log Files**: Recent application logs for debugging

**Usage:**

```bash
# Generate diagnostics for a specific session by ID
biorouter session diagnostics --session-id 20251108_5

# Generate diagnostics for a session by name
biorouter session diagnostics -n my-project-session

# Save diagnostics to a custom location
biorouter session diagnostics --session-id 20251108_5 -o /path/to/my-diagnostics.zip

# Interactive selection (prompts you to choose a session)
biorouter session diagnostics
```

> **Warning.** Diagnostics bundles contain your session messages and system information. If your session includes sensitive data (API keys, personal information, proprietary code), review the contents before sharing publicly.

> **Tip.** Generate diagnostics before reporting bugs to provide technical details that help with faster resolution. The ZIP file can be attached to GitHub issues or shared with support.

### session rename [options]

Rename a saved session. This reads and writes the local session store directly, so it needs no running daemon.

**Options:**

- **`--session-id <session_id>`**: Rename a specific session by ID
- **`-n, --name <name>`**: Rename a specific session by its current name
- **`--path <path>`**: Rename a specific session by file path (legacy)
- **`--new-name <NAME>`** (required): The new name for the session

If you supply no identifier, biorouter prompts you to choose a session. The new name is trimmed, must not be empty, and must be at most 200 characters.

**Usage:**

```bash
# Rename a session by ID
biorouter session rename --session-id 20251108_6 --new-name migration-review

# Rename a session by its current name
biorouter session rename -n old-project --new-name new-project

# Interactive selection (prompts you to choose a session)
biorouter session rename --new-name migration-review
```

### session diverge [options]

Branch a stored conversation into a brand-new session that keeps the history up to the last complete assistant answer. The original session is left untouched. Like `rename`, this works against the local session store and needs no running daemon.

The new session ID is printed to stdout (so it can be captured in a script); the human-readable summary goes to stderr.

**Options:**

- **`--session-id <session_id>`**: Diverge a specific session by ID
- **`-n, --name <name>`**: Diverge a specific session by name
- **`--path <path>`**: Diverge a specific session by file path (legacy)
- **`--branch-name <NAME>`**: Name for the new branched session. Without it, the branch is named after its parent with a `(branch N)` suffix

If you supply no identifier, biorouter prompts you to choose a session.

**Usage:**

```bash
# Branch a session, letting biorouter name the branch
biorouter session diverge --session-id 20251108_6

# Branch a session and name the branch
biorouter session diverge -n migration-review --branch-name try-covering-index

# Resume the branch that was just created
biorouter session --resume --session-id 20251108_7
```

## Task execution

### run [options]

Execute commands from an instruction file or stdin.

**Input Options:**

- **`-i, --instructions <FILE>`**: Path to instruction file containing commands. Use `-` for stdin
- **`-t, --text <TEXT>`**: Input text to provide to biorouter directly
- **`--system <TEXT>`**: Provide additional system instructions to customize the agent's behavior
- **`--workflow <WORKFLOW_NAME_OR_PATH> <OPTIONS>`**: Load a custom workflow in current session
- **`--params <KEY=VALUE>`**: Key-value parameters to pass to the workflow file. Can be specified multiple times
- **`--sub-workflow <WORKFLOW>`**: Specify sub-workflows to include alongside the main workflow. Can be specified multiple times

**Session Options:**

- **`-s, --interactive`**: Continue in interactive mode after processing initial input
- **`-n, --name <name>`**: Name for this run session (e.g. `daily-tasks`)
- **`-r, --resume`**: Resume from a previous run
- **`--path <PATH>`**: Path for this run session (e.g. `./playground.jsonl`). Used for legacy file-based session storage.
- **`--no-session`**: Run biorouter commands without creating or storing a session file

**Extension Options:**

- **`--with-extension <COMMAND>`**: Add stdio extensions (can be used multiple times)
- **`--with-streamable-http-extension <URL>`**: Add remote extensions over Streamable HTTP (can be used multiple times)
- **`--with-builtin <name>`**: Add builtin extensions by name (e.g., 'developer' or multiple: 'developer,github')

**Control Options:**

- **`--debug`**: Output complete tool responses, detailed parameter values, and full file paths
- **`--max-tool-repetitions <NUMBER>`**: Maximum number of times the same tool can be called consecutively with identical parameters. Helps prevent infinite loops
- **`--max-turns <NUMBER>`**: Maximum number of turns allowed without user input (default: 1000)
- **`--explain`**: Show a workflow's title, description, and parameters
- **`--render-workflow`**: Print the rendered workflow instead of running it
- **`-q, --quiet`**: Quiet mode. Suppress non-response output, printing only the model response to stdout
- **`--output-format <FORMAT>`**: Output format (`text`, `json`, or `stream-json`). Default is `text`. Use JSON structured output for automation and scripting: `json` for results after completion, `stream-json` for events as they occur
- **`--provider`**: Specify the provider to use for this session (overrides environment variable)
- **`--model`**: Specify the model to use for this session (overrides environment variable)

**Usage:**

```bash
# Run from instruction file
biorouter run --instructions plan.md

# Load a workflow with a prompt that biorouter executes and then exits  
biorouter run --workflow workflow.yaml

# Load a workflow and stay in an interactive session
biorouter run --workflow workflow.yaml --interactive

# Load a workflow in debug mode
biorouter run --workflow workflow.yaml --debug

# Show workflow details
biorouter run --workflow workflow.yaml --explain

# Run a workflow with parameters
biorouter run --workflow workflow.yaml --params environment=production --params region=us-west-2

# Run instructions from a file without session storage
biorouter run --no-session -i instructions.txt

# Run with a specified provider and model
biorouter run --provider anthropic --model claude-4-sonnet -t "initial prompt"

# Run with limited turns before prompting user
biorouter run --workflow workflow.yaml --max-turns 10
```

### bench

Used to evaluate system-configuration across a range of practical tasks.

**Usage:**

```bash
biorouter bench ...etc.
```

### workflow

Used to validate workflow files, manage workflow sharing, list available workflows, and open workflows in biorouter desktop.

**Commands:**

- **`deeplink <WORKFLOW_NAME>`**: Generate a shareable link for a workflow file
  - **`-p, --param <KEY=VALUE>`**: Pre-fill workflow parameter (can be specified multiple times)
- **`list [OPTIONS]`**: List all available workflows from local directories and configured GitHub repositories
  - **`--format <FORMAT>`**: Output format (`text` or `json`). Default is `text`
  - **`-v, --verbose`**: Show verbose information including workflow titles and full file paths
- **`open <WORKFLOW_NAME>`**: Open a workflow file directly in biorouter desktop
  - **`-p, --param <KEY=VALUE>`**: Pre-fill workflow parameter (can be specified multiple times)
- **`validate <WORKFLOW_NAME>`**: Validate a workflow file

**Usage:**

```bash
# Generate a shareable link
biorouter workflow deeplink my-workflow.yaml

# Generate a deeplink and provide parameter values
biorouter workflow deeplink my-workflow.yaml -p environment=production -p region=us-west-2

# List all available workflows
biorouter workflow list

# List workflows with detailed information
biorouter workflow list --verbose

# List workflows in JSON format for automation
biorouter workflow list --format json

# Open a workflow in biorouter desktop
biorouter workflow open my-workflow.yaml

# Open a workflow by name
biorouter workflow open my-workflow

# Open a workflow and provide parameter value
biorouter workflow open my-workflow --param name=myproject

# Validate a workflow file
biorouter workflow validate my-workflow.yaml

# Get help about workflow commands
biorouter workflow help
```

### schedule

Automate workflows by running them on a [schedule](../workflows/creating-and-sharing-workflows.md#schedule-a-workflow).

**Commands:**

- `add <OPTIONS>`: Create a new scheduled job. Copies the current version of the workflow to the `scheduled_workflows` directory in Biorouter's data directory
- `list`: View all scheduled jobs
- `remove`: Delete a scheduled job
- `sessions`: List sessions created by a scheduled workflow
- `run-now`: Run a scheduled workflow immediately
- `cron-help`: Show cron expression examples and help

**Options:**

- `--schedule-id <NAME>`: A unique ID for the scheduled job (e.g. `daily-report`)
- `--cron "* * * * * *"`: Specifies when a job should run using a [cron expression](https://en.wikipedia.org/wiki/Cron#Cron_expression)
- `--workflow-source <PATH>`: Path to the workflow YAML file
- `-l, --limit <NUMBER>`: Max number of sessions to display when using the `sessions` command

**Usage:**

```bash
biorouter schedule <COMMAND>

# Add a new scheduled workflow which runs every day at 9 AM
biorouter schedule add --schedule-id daily-report --cron "0 0 9 * * *" --workflow-source ./workflows/daily-report.yaml

# List all scheduled jobs
biorouter schedule list

# List the 10 most recent biorouter sessions created by a scheduled job
biorouter schedule sessions --schedule-id daily-report -l 10

# Run a workflow immediately
biorouter schedule run-now --schedule-id daily-report

# Remove a scheduled job
biorouter schedule remove --schedule-id daily-report
```

**Which process makes the change.** `add`, `remove`, `list` and `run-now` go to a running daemon when this terminal can reach one. They find it the way `session send` does: `BIOROUTER_SERVER__SECRET_KEY` and `BIOROUTER_PORT` (default 3000) from this shell, and a daemon answering there. That daemon then makes the change itself, so the schedule is live, listed and deletable in the app at once, and `run-now` runs inside the daemon, where the app's Stop button can reach it. A daemon that rejects the key is reported as an error. The command does not fall back to editing the file behind the daemon's back.

When no daemon can be reached, the command writes `schedule.json` in Biorouter's data directory itself and says so. For example:

```text
No running Biorouter could be reached from this terminal (no daemon answered on 127.0.0.1:3000), so the job was written to the schedule file directly. A Biorouter that is already running — the desktop app included — picks it up from that file within 60 seconds; if none is running, it first runs the next time Biorouter starts.
```

This is always the case next to the desktop app. The app's own daemon uses a random port and a new secret every launch, so a terminal cannot reach it. It is also always the case in an agent's shell, because the daemon's secret is never passed to a tool. A running Biorouter polls the file and usually picks up the change within a couple of seconds, but the promise is 60. `sessions` reads the session store directly and never needs a daemon.

**A change made in the file needs a person.** A scheduled job is a standing unattended agent run, so `add`, `remove` and `run-now` print what will happen and ask for confirmation before writing the file — and refuse, with exit 2 and nothing written, when stdin and stderr are not both terminals. There is no `--yes`: a flag that skipped the question would be a flag an agent could pass. `list` and `sessions` only read, and ask nothing. To make the change from a script, reach a daemon instead: with `BIOROUTER_SERVER__SECRET_KEY` and `BIOROUTER_PORT` pointing at a running one, the daemon makes the change and no terminal is involved.

### mcp

Run an enabled MCP server specified by `<name>` (e.g. `'Google Drive'`). MCP is the Model Context Protocol, the standard biorouter extensions speak.

**Usage:**

```bash
biorouter mcp <name>
```

### acp

Run biorouter as an Agent Client Protocol (ACP) agent server over stdio. This enables biorouter to work with ACP-compatible clients like Zed.

ACP is an emerging protocol specification that standardizes communication between AI agents and client applications, making it easier for clients to integrate with various AI agents.

**Usage:**

```bash
biorouter acp
```

> **Note.** This command is automatically invoked by ACP-compatible clients and is not typically run directly by users. The client manages the lifecycle of the `biorouter acp` process.

## Project management

### project

Start working on your last project or create a new one.

**Alias**: `p`

**Usage:**

```bash
biorouter project
```

### projects

Choose one of your projects to start working on.

**Alias**: `ps`

**Usage:**

```bash
biorouter projects
```

Both `project` and `projects` are interactive and need a terminal. Without one
they exit with status `2` and point at the scriptable equivalent,
`biorouter run --resume --session-id <id> --text "<prompt>"`.

## Interface

### serve

Run Biorouter and reach it from a browser. `serve` starts the `biorouterd` daemon, points it at the built interface, and prints the URL to open — the same interface the desktop app shows, served on one origin with no proxy in the path.

**Alias**: `headless` — the name the retired standalone binary was known by, kept so older instructions still work.

**Options:**

- **`--host <HOST>`**: Address to bind. Anything reachable from another machine requires a token. Default is `127.0.0.1`
- **`-p, --port <PORT>`**: Port to listen on. Default is `8765` — deliberately not `3000`, which is `biorouterd`'s own default
- **`--token <TOKEN>`**: Use this access token instead of generating a fresh one
- **`--no-token`**: Serve without an access token. Refused for a non-loopback bind, and cannot be combined with `--token`
- **`--web-dir <DIR>`**: Directory holding the built interface. Takes precedence over `BIOROUTER_SERVE_UI`; whichever of the two is used must contain an `index.html`, or `serve` refuses to start. Located automatically when neither is set
- **`--open`**: Open a browser once the server is ready

**Usage:**

```bash
# Serve on this machine only, and open a browser
biorouter serve --open

# Serve on a different port
biorouter serve --port 9000

# Reach it from another machine on the network — a token is mandatory here
biorouter serve --host 0.0.0.0

# Reuse one address across restarts, for a bookmark or a service unit
biorouter serve --host 0.0.0.0 --token "$(openssl rand -hex 32)"
```

The printed URL carries an access token as `?t=<token>`, minted per launch and shown once. Opening it exchanges the token for a session cookie and redirects, so the token leaves the address bar; it is not used up, and opens the interface again for anyone who has it until the daemon stops. Use `Ctrl+C` to stop the server, or send `serve` `SIGTERM` (`kill <pid>`); either way it stops the daemon it started and frees the port.

> **Note.** A browser session cannot change its model or provider, deliberately — run `biorouter configure` to choose them **before** starting `serve`. [Reaching Biorouter from a browser](../deployment/browser-access.md) explains why, and covers the access token, remote access and troubleshooting.

### web

> **Deprecated.** Use [`serve`](#serve) instead. `web` serves a minimal standalone chat page rather than the Biorouter interface, and its default port collides with `biorouterd`'s. It is kept for now and unchanged; new deployments should not use it.

Start a new session in biorouter Web, a lightweight web-based interface launched via the CLI that mirrors the desktop app's chat experience.

biorouter Web is particularly useful when:

- You want to access biorouter with a graphical interface without installing the desktop app
- You need to use biorouter from different devices, including mobile
- You're working in an environment where installing desktop apps isn't practical

> **Warning.** Don't expose the web interface to the internet without proper security measures.

**Options:**

- **`-p, --port <PORT>`**: Port number to run the web server on. Default is `3000`
- **`--host <HOST>`**: Host to bind the web server to. Default is `127.0.0.1`
- **`--open`**: Automatically open the browser when the server starts
- **`--auth-token <TOKEN>`**: Require a password to access the web interface

**Usage:**

```bash
# Start web interface at `http://127.0.0.1:3000` and open the browser
biorouter web --open

# Start web interface at `http://127.0.0.1:8080` 
biorouter web --port 8080

# Start web interface accessible from local network at `http://192.168.1.7:8080`
biorouter web --host 192.168.1.7 --port 8080

# Start web interface with authentication required
biorouter web --auth-token <TOKEN>
```

> **Note.** Use `Ctrl+C` to stop the server.

**Limitations:**

While the web interface provides most core features, be aware of these limitations:

- Some file system operations may require additional confirmation
- Extension management must be done through the CLI
- Certain tool interactions might need extra setup
- Configuration changes require a server restart

## Terminal integration

### @biorouter / @g

Ask biorouter questions directly from your shell prompt, with command history included in the context. These aliases are created when you set up terminal integration.

**Examples:**

```bash
# Ask questions with command history context
@biorouter create a python script to process these files
@biorouter create a PR description summarizing these changes
@g how do I fix these permission denied errors?
```

## Interactive session features

### Slash commands

Once you're in an interactive session (via `biorouter session` or `biorouter run --interactive`), you can use these slash commands. All commands support tab completion. Press `/ + <Tab>` to cycle through available commands.

**Available Commands:**

- **`/?` or `/help`** - Display the help menu
- **`/builtin <names>`** - Add builtin extensions by name (comma-separated)
- **`/clear`** - Clear the current chat history
- **`/endplan`** - Exit plan mode and return to 'normal' biorouter mode
- **`/exit` or `/quit`** - Exit the chat
- **`/extension <command>`** - Add a stdio extension (format: ENV1=val1 command args...)
- **`/mode <name>`** - Set the biorouter mode to use ('auto', 'approve', 'chat', 'smart_approve')
- **`/plan <message_text>`** - Enter 'plan' mode with optional message. Create a plan based on the current messages and ask user if they want to act on it
- **`/workflow [filepath]`** - Generate a workflow from the current chat and save it to the specified filepath (must end with .yaml). If no filepath is provided, it will be saved to ./workflow.yaml
- **`/compact`** - Compact and summarize the current chat to reduce context length while preserving key information
- **`/t`** - Toggle between `light`, `dark`, and `ansi` themes. [More info](#themes).
- **`/t <name>`** - Set theme directly (light, dark, ansi)

**Examples:**

```bash
# Create a plan for triaging test failures
/plan let's create a plan for triaging test failures

# Switch to chat mode
/mode chat

# Add a builtin extension during the session
/builtin developer

# Clear the current chat history
/clear
```

You can also create custom slash commands for running workflows in biorouter Desktop or the CLI.

### Themes

The `/t` command controls the syntax highlighting theme for markdown content in biorouter CLI responses. This affects the styles used for headers, code blocks, bold/italic text, and other markdown elements in the response output.

**Commands:**

- `/t` - Cycles through themes: `light` → `dark` → `ansi` → `light`
- `/t light` - Sets `light` theme (subtle light colors)
- `/t dark` - Sets `dark` theme (subtle darker colors)
- `/t ansi` - Sets `ansi` theme (most visually distinct option with brighter colors)

**Configuration:**

- The default theme is `dark`
- The theme setting is saved to the [configuration file](../configuration/config-file-reference.md) as `BIOROUTER_CLI_THEME` and persists between sessions
- The saved configuration can be overridden for the session using the `BIOROUTER_CLI_THEME` [environment variable](../configuration/environment-variables.md#session-management)

> **Note.** Syntax highlighting styles only affect the font, not the overall terminal interface. The `light` and `dark` themes have subtle differences in font color and weight.

The biorouter CLI theme is independent from the biorouter Desktop theme.

**Examples:**

```bash
# Start a named session
biorouter session --name use-custom-theme

# Toggle theme during a session
/t

# Set the light theme during a session
/t light
```

> **Note.** The first example starts a session only; set `BIOROUTER_CLI_THEME` in the environment, as described under Configuration above, to choose the theme for that session.

## Navigation and controls

### Keyboard shortcuts

**Session Control:**

- **`Ctrl+C`** - Interrupt the current request
- **`Ctrl+J`** - Add a newline

**Navigation:**

- **`Cmd+Up/Down arrows`** - Navigate through command history
- **`Ctrl+R`** - Interactive command history search (reverse search). [More info](#command-history-search).

### Command history search

The `Ctrl+R` shortcut provides interactive search through your stored CLI command history. This feature makes it easy to find and reuse recent commands without retyping them. When you type a search term, biorouter searches backwards through your history for matches.

**How it works:**

1. Press `Ctrl+R` in your biorouter CLI session
2. Type a search term
3. Navigate through the results using:
   - `Ctrl+R` to cycle backwards through earlier matches
   - `Ctrl+S` to cycle forward through newer matches
4. Press `Return` (or `Enter`) to run the found command, or `Esc` to cancel

For example, instead of retyping this long command:

```text
analyze the GWAS summary statistics file and suggest follow-up enrichment analyses
```

Use the `"GWAS summary"` or `"enrichment"` search term to find and rerun it.

**Search tips:**

- **Distinctive terms work best**: Choose unique words or phrases to help filter the results
- **Partial matches and multiple words are supported**: You can search for phrases like `"gith"` and `"run the unit test"`

## Related documentation

- [CLI QA checklist](qa-checklist.md) — the manual and headless verification script covering every command on this page.
- [Configuration file reference](../configuration/config-file-reference.md) — the `config.yaml` keys behind `biorouter configure` and the theme setting.
- [Environment variables](../configuration/environment-variables.md) — per-invocation overrides for the same settings, including `BIOROUTER_CLI_THEME`.
- [Managing sessions](../getting-started/managing-sessions.md) — how sessions are stored, resumed, and pruned behind the `session` subcommands.
- [Workspace control](../agent-loop/workspace-control.md) — the daemon routes behind `session watch`, `send`, `attach`, and `cancel`, and the agent-side tools that do the same things.
- [Reaching Biorouter from a browser](../deployment/browser-access.md) — the full guide to `serve`: the access token, reaching it from another machine, and what a browser session cannot do.
- [Creating and sharing workflows](../workflows/creating-and-sharing-workflows.md) — how to author the workflow files that `run --workflow`, `workflow`, and `schedule` consume.
