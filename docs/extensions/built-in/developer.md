# Developer capability

> **What this is.** Two things in one file: a walkthrough of the built-in Developer capability (enabling it, a worked project-setup example, its six tools), and a reference on constraining it with permission modes, tool permissions and `.biorouterignore`.
> **Status:** Current. The capability and its `shell` / `shell_status` / `shell_kill` / `text_editor` / `analyze` / `image_processor` tools ship in `crates/biorouter-mcp/src/developer`, and the permission modes described match `crates/biorouter/src/security`.
> **Audience:** end users, and anyone deciding how much autonomy to grant BioRouter.

The Developer capability lets BioRouter automate developer-centric tasks: editing files, running shell commands, and setting up projects. It also provides enhanced code editing and codebase analysis tools. It is enabled by default when BioRouter is installed.

Because `shell` and `text_editor` can run any command and modify any accessible file with your user privileges, this page carries the most substantive security guidance of any capability page. If you only want to know how to rein BioRouter in, skip to [Configuring access controls](#configuring-access-controls).

## Configuration

> **Note.** The Developer capability is already enabled by default when BioRouter is installed. The steps below are only needed to confirm or restore that state.

1. Run the `configure` command:

   ```bash
   biorouter configure
   ```

2. Choose `Toggle Extensions`, then confirm `developer` is enabled:

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  Toggle Extensions
   │
   ◆  Enable capabilities and extensions: (use "space" to toggle and "enter" to submit)
   │  ● developer
   └  Extension settings updated successfully
   ```

## Example usage

In this example, BioRouter sets up a Python bioinformatics analysis environment with pandas, scikit-learn, scanpy and python-dotenv, and initializes Git.

To follow along, either open a new session in BioRouter Desktop, or start one from a terminal:

```bash
biorouter session
```

> **Note.** This transcript was produced with Anthropic's Claude 4 Sonnet. Any tool-capable model can drive the capability; the exact wording will differ by model.

### BioRouter prompt

```text
set up a new Python bioinformatics project with pandas, scikit-learn, scanpy, and python-dotenv? Fetch the latest package versions, generate a README with those versions, and initialize Git
```

After about a minute, the whole analysis environment was set up.

### BioRouter output

```md
I'll help you set up a new Python bioinformatics project with those dependencies. I'll break this down into steps:

1. First, let's create a new directory and initialize the project
2. Then install the dependencies
3. Create a detailed README
4. Initialize git
5. Create some starter files
6. Let's begin:

[... shell commands, files created ...]

Great! I've set up a complete Python analysis project with pandas, scikit-learn, and scanpy integrations:

1. Created a new project directory and initialized a virtual environment
2. Installed pandas, scikit-learn, scanpy, and python-dotenv dependencies
3. set up the basic project structure with directories for:
    - config/
    - data/
    - notebooks/ 
    - pipelines/
4. Created essential files:
    - run_analysis.py (main analysis entry point)
    - .env (environment variables, e.g. data paths)
    - .gitignore (with common Python ignores)
5. Added a basic cohort loader with sample_id and diagnosis fields
6. Created an analysis module with two functions:
    - load_cohort (read and validate the cohort table)
    - run_qc (apply quality-control filters)
7. Added pipeline scripts under `pipelines/`
8. Updated requirements.txt with pinned versions
9. Created a comprehensive README.md with:
    - Installation steps
    - Project structure
    - Available scripts
    - Environment variables 
    - Latest package versions

The project structure is:

cohort-analysis/
├── config/         # Configuration files
├── data/           # Input datasets (gitignored)
├── notebooks/      # Exploratory analysis notebooks
│   └── eda.ipynb
├── pipelines/      # Analysis pipelines
│   └── qc_pipeline.py
├── .env            # Environment variables
├── .gitignore      # Git ignore file
├── run_analysis.py # Entry point
└── requirements.txt # Project dependencies

To start developing you can:

1. Run `pip install -r requirements.txt` to install dependencies
2. run the analysis with `python run_analysis.py` 

For example, to load a cohort and run quality control:

python run_analysis.py --cohort data/cohort.csv --step qc

Would you like me to add any additional features or make other improvements to the setup?
```

## Foreground and background commands

The `shell` tool runs a command one of two ways, and the difference matters more than it looks.

**Foreground** is the default: the command blocks the turn until it finishes, and its output comes back as the tool result. It carries a wall-clock budget, 240 seconds by default. When the budget expires BioRouter kills the command's **whole process group** — not just the shell it launched — and the call fails with an error that names the command, how long it ran, and what to do instead, followed by what the command printed before the kill (its last 100 lines, at most 8,000 characters). Nothing is left running. The error is marked not retryable, because the same command would be killed again at the same point, so keep each foreground command, any `sleep` included, well under the budget. The tool description says so too: under about 200 seconds at the default budget.

A foreground command that opens with a plain `sleep N` the shell waits for (`sleep 240`, `sleep 240; tail log`, `sleep 4m && ls`), where `N` is at least the budget, is refused before it runs, with an `INVALID_PARAMS` error that starts with "Not run" and suggests a shorter poll or `background=true`. It would only have spent the whole budget to be killed inside the sleep. Only that shape is read: `sleep 300 &`, `cd x && sleep 300`, `sleep $DELAY`, a pipe or a redirection run as before, and a `background=true` call has no foreground budget to refuse by.

A foreground command is finished when its shell exits. If the shell started something in the background with `&` that still holds the command's output (`cd dir && nohup job > log 2>&1 &` under bash, or `nohup job &` under any shell), the call waits two seconds for the output to close and then returns what was printed, with a note saying so. The process it started keeps running. Anything it prints to that output is discarded, and the output is closed 30 seconds after the shell exited, which can stop a process that writes to it later. Start long work with `background=true`, or send its output to a file and read the file.

**Background** is `background=true`. The call returns a `job_id` and the job keeps running across tool calls, watched or peeked at with `shell_status` (omit `job_id` to list every job; give `job_id` for one job's status and the output since the last look; add `timeout_secs` to block until it exits), and stopped with `shell_kill`. There is no budget. This is where a dev server, a build, a test suite or a long training run belongs.

### Reading a background job

A background job can be followed with the shell alone, because a roster may grant `shell` without `shell_status` (issue #538):

- **The start reply waits up to 2 seconds.** A job that ends in that time is reported like a foreground command: its status, its output, and for a non-zero exit or a signal the `[shell: command exited with status N]` line, an error result and a `tool_failure` envelope. A job still running gets "Started background job job-N." as the first sentence, then its process group and its log file. The wait is the cost of reporting an instant failure; every start of a job that keeps running takes 2 seconds longer.
- **Every job writes a log file.** It lives in the temp directory, readable only by you (mode 0600 on macOS and Linux), with a name no other job or chat shares, and the reply names it with a command that reads its end (`tail -n 50 <path>`). The file holds the job's raw output from both stdout and stderr. Once the operating system reports how the job ended, Biorouter appends one last line such as `[biorouter: background job job-1 exited with status 0]`. A line the job printed itself never ends the job or changes its status. A log stops at 16 MiB, and all logs in one Biorouter process together stop at 256 MiB; a log that stops says so, and still gets its last line. The file is removed when the chat's Developer capability shuts down, or, for a job still running then, when that job finishes. A crash can leave files behind in the temp directory, as it can for long foreground output.
- **`shell_status` returns what a terminal would show** (issue #528). A line a progress bar rewrote with carriage returns comes back as its final state, an unfinished line is shown up to its last whole character and is not repeated while it stays the same, and one result is at most 100 lines and 8,000 characters. Longer output comes back as its first and last lines, with the path of a private file that holds the raw text the result covered. Each job keeps its four most recent such files.
- **A job whose shell will not exit gets an advisory** (issue #523). In a pipeline the shell waits for every stage, so a process the job started that outlives its parent and holds a stage's input open keeps the job running. When a check of a running job finds no new output, `shell_status` reads the process table once (bounded to one second) and names any member of the job's own process group whose parent left the job. The job stays `running`, with no exit status, because none exists yet; nothing is signalled. `shell_kill` stops the job's own process group.

The budget exists because a foreground command that turns out to be far more expensive than it looked — a `find` over a whole home directory, a query with no index — blocks the turn for minutes with nothing to show for it. Two things make that visible while it happens:

- The tool card in chat reports the elapsed time every 15 seconds ("shell: still running after 45s — …"), so a silent command is distinguishable from a stuck agent.
- The command is listed in the active-work view (`GET /active_work`) for as long as it runs, alongside background jobs, subagents and scheduled runs, and `POST /active_work/{id}/cancel` stops it on its own without ending the turn. The row names the chat that ran the command, as a background job's row does. The view shows it only to a caller that could open that chat, and only such a caller can stop it; see [Reaching a private chat from a script](../../deployment/programmatic-session-access.md).

Raise, lower or disable the budget with [`BIOROUTER_SHELL_FOREGROUND_TIMEOUT_SECS`](../../configuration/environment-variables.md#foreground-shell-budget) (seconds; `0` disables it).

> **Stop reaps the whole tree.** Pressing Stop, cancelling the turn, or hitting the budget terminates the command's process group, so anything the command itself spawned goes with it. This holds for a command the model runs directly and for one it runs from inside a [Code Execution](code-execution.md) script.

## Configuring access controls

By default, BioRouter can run system commands with your user privileges and edit any accessible file **without your approval**. This is because BioRouter runs in Autonomous permission mode by default and has access to the Developer capability's shell and file editing tools. While this configuration lets BioRouter work quickly and independently, there is potential for unexpected outcomes. Understanding the available access control features helps you configure BioRouter to match your comfort level and specific needs.

> **Tip.** See the [Quick setup example](#quick-setup-example) below for ways to configure more control over BioRouter's behavior.

### Developer capability tools

The Developer capability provides these tools:

| Tool | Description | Use Cases | Risk Level |
|------|-------------|-----------|------------|
| `shell` | Execute shell commands | Running tests, installing packages, git operations | ⚠️ High<br />Can run any system command with your user privileges |
| `shell_status` | Inspect a running shell job | Check progress and collect output | Read-only job status |
| `shell_kill` | Stop a shell job | Cancel work started by the shell tool | Terminates the selected job |
| `text_editor` | Read, write, and edit files | Code refactoring, creating files, updating configs | ⚠️ High<br />Can modify any accessible file |
| `analyze` | Analyze code structure | Understanding codebase, finding dependencies | ✅ Low<br />Read-only code analysis |
| `image_processor` | Process and resize images | Optimizing assets, format conversion | ✅ Low<br />Image manipulation only |

Desktop observation and control belong exclusively to [Biorouter Copilot](computer-controller.md),
which obtains a separate grant for each user request. Developer does not supply a desktop
screenshot tool or an alternate native-control route.

### Access control features

You can layer multiple controls to match your risk tolerance and workflow.

**[Permission modes](../../security/permission-modes.md)** control when BioRouter asks for approval:

| Mode | Description | Use Cases |
|------|-------------|-----------|
| Autonomous<br />CLI: `auto` | No approval required | Best for experienced users in safe environments |
| Manual Approval<br />CLI: `approve` | Review every action | Recommended for sensitive work or when you want maximum control |
| Smart Approval<br />CLI: `smart_approve` | AI decides what needs review | Balanced approach |
| Chat Only<br />CLI: `chat` | Disable all tools | For maximum security and models that don't support tool-calling |

**Tool permissions** let you set `Always allow`, `Ask before`, and `Never allow` permissions for individual capability or extension tools when in Manual Approval or Smart Approval modes.

**`.biorouterignore` files** restrict which files and directories BioRouter can access (`.gitignore` files are the fallback).

> **Tip.** You can change permission modes during a session without restarting. In the CLI, use the `/mode` command (for example `/mode approve`). In Desktop, use the mode selector button in the bottom menu.

#### Quick setup example

You might want more control over BioRouter's operations when working with sensitive systems, exploring unfamiliar codebases, using untrusted models, or simply preferring to review actions before execution.

Here's an example configuration that enables oversight:

1. **Set the [permission mode](../../security/permission-modes.md)** to Smart Approval or Manual Approval:

   ```yaml
   # ~/.config/biorouter/config.yaml
   BIOROUTER_MODE: smart_approve  # or approve
   ```

2. **Create a `.biorouterignore` file** in your project to protect sensitive files:

   ```text
   .env*
   secrets.*
   *.key
   *.pem
   .git/
   ```

3. **Configure tool permissions** based on your needs.

As you become more comfortable with BioRouter's behavior, you can adjust these settings to reduce friction while maintaining appropriate safeguards for your environment.

## Related documentation

- [Security guide](../../security/README.md) — the umbrella page for using BioRouter safely.
- [Permission modes](../../security/permission-modes.md) — the full reference for approval modes and per-tool permissions.
- [Biorouter Copilot capability](computer-controller.md) — the other high-privilege built-in capability; it automates your desktop rather than your project, and the same access controls apply.
- [Code Execution capability](code-execution.md) — Code Mode scripts import `shell` and `text_editor` from this capability, so they inherit its blast radius.
- [Configuration file reference](../../configuration/config-file-reference.md) — where `BIOROUTER_MODE` and other settings live in `config.yaml`.
