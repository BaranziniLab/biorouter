# Creating and sharing workflows

> **What this is.** A task-oriented walkthrough of the workflow lifecycle: turning a live session into a reusable workflow, editing it, running it, validating it, sharing it by link or file, and putting it on a schedule.
> **Status:** Current — the `/workflow` slash command and the `biorouter workflow` and `biorouter schedule` subcommands described here match the shipped Desktop and CLI behaviour.
> **Audience:** end users who have run something in Biorouter once and want to run it again, or hand it to someone else.

Sometimes you finish a task in Biorouter and realize that the setup could be useful again. Maybe you have curated a good combination of tools, defined a clear goal, and want to preserve that flow. Or maybe you want to help someone else replicate what you just did without walking them through it step by step.

You can turn your current Biorouter session into a reusable workflow that includes the tools, goals, and setup you're using right now, and package it into a workflow file that others (or future you) can launch with a single click.

Every step below is given twice where the two interfaces differ: once for **Biorouter Desktop** and once for the **Biorouter CLI**. Follow only the path for the interface you are using.

## Contents

- [Create a workflow](#create-a-workflow)
  - [By asking Biorouter](#by-asking-biorouter)
- [Edit a workflow](#edit-a-workflow)
- [Use a workflow](#use-a-workflow)
- [Validate a workflow](#validate-a-workflow)
- [Share a workflow](#share-a-workflow)
- [Schedule a workflow](#schedule-a-workflow)
- [Core components](#core-components)
- [Advanced features](#advanced-features)
- [What a workflow includes](#what-a-workflow-includes)

## Create a workflow

Create a workflow from the current session or from a template.

### In Desktop, from the current session

1. While in the session you want to save as a workflow, open the **chat summary** (the ☰ button in the chat header, beside the terminal button) and click **Make workflow**.
2. In the dialog that opens, review and edit the workflow fields as needed. Biorouter fills it in from the conversation, and captures the session's extensions, knowledge bases and model pin for you.
3. When you're finished, you can:
   - Click `Create Workflow` to save the workflow to your Workflow Library
   - Click `Create & Run Workflow` to save and immediately run the workflow in a new session

### By asking Biorouter

You can also just ask, in any interface:

> Turn this chat into a workflow.
> What workflows do I have?
> Delete the old weekly report workflow.
> Run the cohort summary workflow every Monday at 8am.

Biorouter drafts the workflow from the conversation and shows it to you before anything is saved. Saving, deleting, importing and scheduling each ask you to approve the change first — Biorouter will not alter your workflow library on its own.

> **Note.** This is unavailable in a browser session started by `biorouter serve`, which has no way to prove a person approved a change. Listing and reading still work there.

### In Desktop, from the Workflow Library

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` in the sidebar.
3. Click `Create Workflow`.
4. In the dialog that opens, fill in the workflow fields as needed.
5. When you're finished, you can:
   - Copy the workflow link to share the workflow with others
   - Click `Save Workflow` to save the workflow to your Workflow Library
   - Click `Save & Run Workflow` to save and immediately run the workflow in a new session

> **Warning.** You cannot create a workflow from an existing workflow session, but you can view or [edit the workflow](#edit-a-workflow).

### In the CLI, from the current session

Workflow files can be either JSON (`.json`) or YAML (`.yaml`) files. While in a [session](../getting-started/managing-sessions.md), run this command to save the conversation as a workflow:

```sh
/workflow
```

With no argument it goes into your **workflow library** (`~/.config/biorouter/workflows/`), under a filename derived from its title, where `biorouter workflow list` and the Desktop library can both see it. It captures the same extensions, knowledge bases and model pin the Desktop dialog does.

> **Note.** Before Biorouter 1.90 this wrote `workflow.yaml` into whatever directory the CLI happened to be started in, where nothing could list it, and it skipped the session capture. Pass a path if you still want a file in a specific place.

If you want to write it somewhere specific, give a path:

```sh
/workflow my-custom-workflow.yaml
```

**Workflow file structure:**

```yaml
   # Required fields
   version: 1.0.0
   title: $title
   description: $description
   instructions: $instructions    # Define the model's behavior

   # Optional fields
   prompt: $prompt                # Initial message to start with
   extensions:                    # Tools the workflow needs
   - $extensions
   activities:                    # Example prompts to display in the Desktop app
   - $activities
   settings:                      # Additional settings
     biorouter_provider: $provider    # Provider to use for this workflow
     biorouter_model: $model          # Specific model to use for this workflow
     temperature: $temperature    # Model temperature setting for this workflow (0.0 to 1.0)
   retry:                         # Automated retry logic with success validation
     max_retries: $max_retries    # Maximum number of retry attempts
     checks:                      # Success validation checks
     - type: shell
       command: $validation_command
     on_failure: $cleanup_command # Optional cleanup command on failure
```

For detailed descriptions and example configurations of all workflow fields, see the [workflow schema reference](workflow-schema-reference.md).

> **Warning.** You cannot create a workflow from an existing workflow session — the `/workflow` command will not work.

> **Tip.** You should [validate your workflow](#validate-a-workflow) to verify that it's complete and properly formatted.

#### Optional parameters

You may add parameters to a workflow, which will require users to fill in data when running the workflow. Parameters can be added to any part of the workflow (instructions, prompt, activities, etc).

To use parameters:

1. Add template variables using `{{ variable_name }}` syntax in your workflow content
2. Define each parameter in the `parameters` section of your YAML file

**Example workflow with parameters:**

```yaml
   version: 1.0.0
   title: "{{ project_name }} Code Review" # Wrap the value in quotes if it starts with template syntax to avoid YAML parsing errors
   description: Automated code review for {{ project_name }} with {{ language }} focus
   instructions: You are a code reviewer specialized in {{ language }} development.
   prompt: |
      Apply the following standards:
      - Complexity threshold: {{ complexity_threshold }}
      - Required test coverage: {{ test_coverage }}%
      - Style guide: {{ style_guide }}
   activities:
   - "Review {{ language }} code for complexity"
   - "Check test coverage against {{ test_coverage }}% requirement"
   - "Verify {{ style_guide }} compliance"
   settings:                     
     biorouter_provider: "anthropic"   
     biorouter_model: "claude-3-7-sonnet-latest"          
     temperature: 0.7 
   parameters:
   - key: project_name
     input_type: string
     requirement: required # could be required, optional or user_prompt
     description: name of the project
   - key: language
     input_type: string
     requirement: required
     description: language of the code
   - key: complexity_threshold
     input_type: number
     requirement: optional
     default: 20 # default is required for optional parameters
     description: a threshold that defines the maximum allowed complexity
   - key: test_coverage
     input_type: number
     requirement: optional
     default: 80
     description: the minimum test coverage threshold in percentage
   - key: style_guide
     input_type: string
     description: style guide name
     requirement: user_prompt
     # If style_guide param value is not specified in the command, user will be prompted to provide a value, even in non-interactive mode
```

See the [workflow schema reference](workflow-schema-reference.md#parameters) for more information about workflow fields.

### With the online Workflow Generator

Use the online [Workflow Generator](http://biorouter.ucsf.edu/workflow-generator) tool to create a workflow. First choose your preferred format:

- **URL format**: Generates a shareable link that opens a session in Biorouter Desktop
- **YAML format**: Generates YAML content that you can save to file and then run in the Biorouter CLI

Then fill out the workflow form by providing:

- A **title** for the workflow
- A **description**
- A set of **instructions** for the workflow
- An optional initial **prompt**:
  - In the Desktop, the prompt displays in the chat box.
  - In the CLI, the prompt provides the initial message to run. Note that a prompt is required to run the workflow in headless (non-interactive) mode.
- A set of optional **activities** to display in the Desktop
- YAML format only: Optional **author** contact information and **extensions** the workflow uses

## Edit a workflow

### In Desktop

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` in the sidebar.
3. Find the workflow you want to edit and click the edit button.
4. In the dialog that appears, edit any of the workflow fields.
5. When you're finished, you can:
   - Copy the workflow link to share the workflow with others
   - Click `Save Workflow` to save your changes
   - Click `Save & Run Workflow` to save and immediately run the workflow in a new session

> **Tip.** You can also access the edit dialog while using a workflow in a session: just click the edit button at the bottom of the app. The button shows up after you've sent your first message.

### In the CLI

Once the workflow file is created, you can open it with your preferred text editor and modify the value of any field.

## Use a workflow

### In Desktop

1. Open the workflow using a direct link or manual URL entry, or from your Workflow Library:

   **Direct link:** click a workflow link shared with you.

   **Manual URL entry:**
   1. Paste a workflow link into your browser's address bar.
   2. Press `Enter` and click the `Open Biorouter.app` prompt.

   **Workflow Library:**
   1. Click the sidebar button in the top-left to open the sidebar.
   2. Click `Workflows` in the sidebar.
   3. Find your workflow in the Workflow Library.
   4. Click `Use` next to the workflow you want to open.

   **Slash command:** enter a [custom slash command](../agent-loop/context-engineering.md) in any Biorouter chat session.

2. The first time you run a workflow, a warning dialog displays the workflow's title, description, and instructions for you to review. If you trust the workflow content, click `Trust and Execute` to continue. You won't be prompted again for the same workflow unless it changes.

3. If the workflow contains parameters, enter your values in the `Workflow Parameters` dialog and click `Start Workflow`.

   Parameters are dynamic values used in the workflow:

   - **Required parameters** are marked with red asterisks (*)
   - **Optional parameters** show default values that can be changed

4. To run the workflow, click an activity bubble or send the prompt.

> **Note.** Each person gets their own private session, no data is shared between users, and your session won't affect the original workflow creator's session.

### In the CLI

Using a workflow with the Biorouter CLI might involve the following tasks:

- [Configuring your workflow location](#configure-workflow-location)
- [Running a workflow](#run-a-workflow)
- [Scheduling a workflow](#schedule-a-workflow)

#### Configure workflow location

Workflows can be stored locally on your device or in a GitHub repository. Configure your workflow repository using either the `biorouter configure` command or the [config file](../configuration/config-file-reference.md#global-settings).

> **Tip.** Each workflow should be in its own directory, the directory name should match the workflow name you use in commands, and the workflow file can be either `workflow.yaml` or `workflow.json`.

**Using `biorouter configure`:**

```sh
biorouter configure
```

You'll see the following prompts:

```text
┌  biorouter-configure 
│
◆  What would you like to configure?
│  ○ Configure Providers
│  ○ Custom Providers
│  ○ Add Extension
│  ○ Toggle Extensions
│  ○ Remove Extension
// highlight-start
│  ● Biorouter settings (Set the mode, tool output, tool permissions, experiments, the workflow GitHub repository and more)
// highlight-end
│
◇  What would you like to configure?
│  Biorouter settings
│
◆  What setting would you like to configure?
│  ○ Biorouter mode
│  ○ Tool Permission
│  ○ Tool Output
│  ○ Max Turns
│  ○ Lead/Worker Model
│  ○ Secret Storage
│  ○ Toggle Experiment
// highlight-start
│  ● Workflow GitHub repository (Where Biorouter looks for a workflow it does not find on this computer)
// highlight-end
└  
┌  biorouter-configure 
│
◇  What would you like to configure?
│  Biorouter settings
│
◇  What setting would you like to configure?
│  Workflow GitHub repository
│
◆  Enter your biorouter workflow GitHub repo (owner/repo): eg: my_org/biorouter-workflows
// highlight-start
│  BaranziniLab/biorouter-workflows
// highlight-end
└  
```

**Using the config file** — add to `~/.config/biorouter/config.yaml`:

```yaml
BIOROUTER_WORKFLOW_GITHUB_REPO: "owner/repo"
```

#### Run a workflow

##### From a local file

**Basic usage** — run once and exit (see [run options](../cli/command-reference.md#run-options) and [workflow commands](../cli/command-reference.md#workflow) for more):

```sh
# Using workflow file in current directory or BIOROUTER_WORKFLOW_PATH directories
biorouter run --workflow workflow.yaml

# Using full path
biorouter run --workflow ./workflows/my-workflow.yaml
```

`BIOROUTER_WORKFLOW_PATH` is documented in the [environment variables guide](../configuration/environment-variables.md#workflow-configuration).

**Preview a workflow** — use the [`explain` option](../cli/command-reference.md#run-options) to view details before running.

**Interactive mode** — start an interactive session:

```sh
biorouter run --workflow workflow.yaml --interactive
```

The interactive mode will prompt for required values:

```text
◆ Enter value for required parameter 'language':
│ Python
│
◆ Enter value for required parameter 'style_guide':
│ PEP8
```

**With parameters** — supply parameter values when running workflows. See the [`run` command documentation](../cli/command-reference.md#run-options) for detailed examples and options.

Basic example:

```sh
biorouter run --workflow workflow.yaml --params language=Python
```

**Slash command** — enter a [custom slash command](../agent-loop/context-engineering.md) in any Biorouter chat session.

##### From a configured GitHub repository

Once you've configured your GitHub repository, you can run workflows by name.

**Basic usage** — run workflows from your configured repo using the workflow name that matches its directory (see [run options](../cli/command-reference.md#run-options) and [workflow commands](../cli/command-reference.md#workflow) for more):

```sh
biorouter run --workflow workflow-name
```

For example, if your repository structure is:

```text
my-repo/
├── code-review/
│   └── workflow.yaml
└── setup-project/
    └── workflow.yaml
```

You would run the following command to run the code review workflow:

```sh
biorouter run --workflow code-review
```

**Preview a workflow** — use the [`explain` option](../cli/command-reference.md#run-options) to view details before running.

**Interactive mode** — with parameter prompts:

```sh
biorouter run --workflow code-review --interactive
```

The interactive mode will prompt for required values:

```text
◆ Enter value for required parameter 'project_name':
│ MyProject
│
◆ Enter value for required parameter 'language':
│ Python
```

**With parameters** — supply parameter values when running workflows. See the [`run` command documentation](../cli/command-reference.md#run-options) for detailed examples and options.

> **Note.** Each person gets their own private session, no data is shared between users, and your session won't affect the original workflow creator's session. The CLI can prompt users for required [extension secrets](workflow-schema-reference.md#extension-secrets).

## Validate a workflow

Workflow validation is only available through the CLI.

Validate your workflow file to ensure it's properly configured. Validation verifies that:

- All required fields are present
- Parameters are properly formatted
- Referenced extensions exist and are valid
- The YAML/JSON syntax is correct

```sh
biorouter workflow validate workflow.yaml
```

> **Note.** If you want to validate a workflow you just created, you need to exit the session before running the [`validate` subcommand](../cli/command-reference.md#workflow). See [Managing sessions](../getting-started/managing-sessions.md).

Workflow validation can be useful for:

- Troubleshooting workflows that aren't working as expected
- Verifying workflows after manual edits
- Automated testing in CI/CD pipelines

## Share a workflow

Share your workflow with Biorouter users using a workflow link or workflow file.

> **Note.** Each recipient gets their own private session when using your shared workflow. No data is shared between users, and your original session and workflow remain unaffected.

### Share via workflow link

You can share a workflow with Desktop users via a workflow link.

**In Desktop** — copy the deeplink from your Workflow Library to share with others:

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` in the sidebar.
3. Find the workflow you want to share and click the copy-link button to copy the link.

**In the CLI** — generate a deeplink from your workflow file to share with others:

```sh
biorouter workflow deeplink <FILE>
```

You can also provide parameter values to pre-fill the `Workflow Parameters` dialog:

```sh
biorouter workflow deeplink <FILE> --param key1=value1 --param key2=value2
```

When someone clicks the link, it will open Biorouter Desktop with your workflow configuration. They can also use your workflow link to [import a workflow](workflow-storage-and-discovery.md#importing-workflows) for future use.

### Share via workflow file

You can share a workflow with Desktop or CLI users by sending the workflow file directly.

- Desktop users can [import the workflow](workflow-storage-and-discovery.md#importing-workflows) (YAML only).
- CLI users can run a YAML or JSON workflow using `biorouter run --workflow <FILE>` or open it directly in Biorouter Desktop with `biorouter workflow open <FILE>`. See the [CLI command reference](../cli/command-reference.md#workflow) for details.

## Schedule a workflow

Automate Biorouter workflows by running them on a schedule. [Scheduled jobs](scheduled-jobs.md) covers the scheduler itself — cron syntax, headless requirements, and job management — in more depth.

### In Desktop

When creating a schedule, you'll configure:

- **Name**: A descriptive name for the schedule
- **Source**: The workflow to run
- **Execution mode**: Whether the workflow runs in the background (no window, results saved) or foreground (opens window if Biorouter Desktop is running, otherwise runs in background)
- **Frequency and time**: When to run the workflow (e.g. every 20 minutes, weekly at 10 AM on Friday). Your selection is converted into a [cron expression](https://en.wikipedia.org/wiki/Cron#Cron_expression) used by Biorouter.

**Schedule from the Workflow Library:**

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` in the sidebar.
3. Find the workflow you want to schedule and click the schedule button.
4. Click `Create Schedule`.
5. In the dialog that appears, configure the schedule. For **Source**, your workflow link is already provided.
6. Click `Create Schedule`.

**Schedule from the Scheduler view:**

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Scheduler`.
3. Click `Create Schedule`.
4. In the dialog that appears, configure the schedule. For **Source**, select a `.yaml` or `.yml` file or provide a [workflow link](#share-a-workflow).
5. Click `Create Schedule`.

**Manage scheduled workflows:**

Your scheduled workflows are listed in the `Scheduler` page. Click on a schedule to view details, see when it was last run, and perform actions with the scheduled workflow:

- `Run Schedule Now` to trigger the workflow manually
- `Edit Schedule` to change the scheduled frequency
- `Pause Schedule` to stop the workflow from running automatically

At the bottom of the `Schedule Details` page you can view the list of sessions created by the scheduled workflow and open or restore each session.

### In the CLI

Automate Biorouter workflows by scheduling them to run with a [cron expression](https://en.wikipedia.org/wiki/Cron#Cron_expression).

```bash
# Add a new scheduled workflow which runs every day at 9 AM
biorouter schedule add --schedule-id daily-report --cron "0 0 9 * * *" --workflow-source ./workflows/daily-report.yaml
```

You can use either a 5, 6, or 7-digit cron expression for full scheduling precision, following the format "seconds minutes hours day-of-month month day-of-week year".

See the [`schedule` command documentation](../cli/command-reference.md#schedule) for detailed examples and options.

## Core components

A workflow needs these core components:

- **Instructions**: Define the agent's behavior and capabilities
  - Acts as the agent's mission statement
  - Makes the agent ready for any relevant task
  - Required if no prompt is provided

- **Prompt** (optional): Starts the conversation automatically
  - Without a prompt, the agent waits for user input
  - Useful for specific, immediate tasks
  - Required if no instructions are provided

- **Activities**: Example tasks that appear as clickable bubbles
  - Help users understand what the workflow can do
  - Make it easy to get started

## Advanced features

### Automated retry logic

Workflows can include retry logic to automatically attempt task completion multiple times until success criteria are met. This is particularly useful for:

- **Automation workflows** that need to ensure successful completion
- **Development tasks** like running tests that may need multiple attempts
- **System operations** that require validation and cleanup

**Basic retry configuration:**

```yaml
retry:
  max_retries: 3
  checks:
    - type: shell
      command: "test -f output.txt"  # Check if output file exists
  on_failure: "rm -f temp_files*"   # Cleanup on failure
```

**How it works:**

1. Workflow executes normally with provided instructions
2. After completion, success checks validate the results
3. If validation fails and retries remain:
   - Optional cleanup command runs
   - Agent state resets to initial conditions
   - Workflow execution starts over
4. Process continues until either success or max retries reached

See the [workflow schema reference](workflow-schema-reference.md#retry) for complete retry configuration options and examples.

### Structured output for automation

Workflows can enforce [structured JSON output](workflow-schema-reference.md#response), making them ideal for automation workflows that need to parse and process agent responses reliably. Key benefits include:

- **Reliable parsing**: Consistent JSON format for scripts, automation, and CI/CD pipelines
- **Built-in validation**: Ensures output matches your requirements
- **Easy extraction**: Final output appears as a single line for simple parsing

Structured output is particularly useful for:

- **Development workflows**: Code analysis reports, test results with pass/fail counts, and build status with deployment readiness
- **Data processing**: Results with counts and validation status, content analysis with structured findings
- **Documentation generation**: Consistent metadata and structured project reports for further processing

**Example structured output configuration:**

```yaml
response:
  json_schema:
    type: object
    properties:
      build_status:
        type: string
        enum: ["success", "failed", "warning"]
        description: "Overall build result"
      tests_passed:
        type: number
        description: "Number of tests that passed"
      tests_failed:
        type: number
        description: "Number of tests that failed"
      artifacts:
        type: array
        items:
          type: string
        description: "Generated build artifacts"
      deployment_ready:
        type: boolean
        description: "Whether the build is ready for deployment"
    required:
      - build_status
      - tests_passed
      - tests_failed
      - deployment_ready
```

**How it works:**

1. Workflow runs normally with provided instructions
2. Biorouter calls a `final_output` tool with JSON matching your schema
3. Output is validated against the JSON schema
4. If validation fails, Biorouter receives error details and must correct the output
5. Final validated JSON appears as the last line of output for easy extraction

**Example automation usage:**

```bash
# Run workflow and extract JSON output
biorouter run --workflow analysis.yaml --params project_path=./src > output.log
RESULT=$(tail -n 1 output.log)
echo "Analysis Status: $(echo $RESULT | jq -r '.build_status')"
echo "Issues Found: $(echo $RESULT | jq -r '.tests_failed')"
```

> **Note.** Structured output is supported in workflows run in both the Biorouter CLI and Biorouter Desktop. However, creating and editing the `json_schema` configuration must be done manually in the workflow file.

## What a workflow includes

A workflow captures:

- AI instructions (goal/purpose)
- Suggested activities (examples for the user to click)
- Enabled extensions and their configurations
- Project folder or file context
- Initial setup (but not full conversation history)
- The model and provider to use when running the workflow (optional)
- Retry logic and success validation configuration (if configured)

To protect your privacy and system integrity, Biorouter excludes:

- Global and local memory
- API keys and personal credentials
- System-level Biorouter settings

This means others may need to supply their own credentials or memory context if the workflow depends on those elements.

## Related documentation

- [Workflows](README.md) — the folder index and a quick summary of the file format.
- [Workflow schema reference](workflow-schema-reference.md) — the authoritative field-by-field spec behind every example on this page.
- [Workflow storage and discovery](workflow-storage-and-discovery.md) — where the files you create here are saved, and how to find them again.
- [Scheduled jobs](scheduled-jobs.md) — the scheduler in full, including cron syntax and headless requirements.
- [biorouter CLI command reference](../cli/command-reference.md#workflow) — every flag for `run --workflow`, `workflow`, and `schedule`.
