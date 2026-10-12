# Workflow storage and discovery

> **What this is.** Where Biorouter workflow files live on disk, how to save and import them, and how to list and find them again from the Desktop Workflow Library or the CLI.
> **Status:** Current.
> **Audience:** end users who have written workflows and need to keep, organize, and re-find them.

Biorouter Desktop and the Biorouter CLI take different approaches to storage, and knowing which one you are using determines where a workflow ends up:

- **Biorouter Desktop** has a visual Workflow Library for browsing and managing saved workflows.
- **Biorouter CLI** stores workflows as files that you find using file paths or environment variables.

## Understanding workflow storage

Where a workflow is stored determines which projects and sessions can use it.

### Workflow storage locations

| Type | Location | Availability | Best For |
|------|----------|-------------|----------|
| **Global** | `~/.config/biorouter/workflows/` | All projects and sessions | Personal workflows, general-purpose workflows |
| **Local** | `YOUR_WORKING_DIRECTORY/.biorouter/workflows/` | Only when working in that project | Project-specific workflows, team workflows |

**Choose global storage when:**
- You want the workflow available across all projects
- It's a personal workflow or general-purpose workflow
- You're the primary user of the workflow

**Choose local storage when:**
- The workflow is specific to a particular project
- You're working with a team and want to share the workflow
- The workflow depends on project-specific files or configurations

> **Note.** These two locations are where workflows are *saved*. They are not the whole set of places Biorouter *looks*: [the workflows index](README.md#workflow-locations) also lists `BIOROUTER_WORKFLOW_PATH` and a configured GitHub repository. The full search order used by `biorouter workflow list` is under [Workflow discovery process](#workflow-discovery-process) below.

## Storing workflows

### In Desktop

**Save a new workflow:**

1. To create a workflow from your chat session, see [Create a workflow](creating-and-sharing-workflows.md#create-a-workflow).
2. Once in the Workflow Editor, click `Save Workflow` to save it to your Workflow Library.

**Save a modified workflow:**

If you're already using a workflow and want to save a modified version:

1. Click the edit button at the bottom of the app, which appears after sending your first message.
2. Make any desired edits to the instructions, prompt, or other fields.
3. Click `Save Workflow`.

> **Note.** When you modify and save a workflow with a new name, a new workflow and new link are generated. You can still run the original workflow from the workflow library, or using the original link. If you edit a workflow without changing its name, the version in the workflow library is updated, but you can still run the original workflow via link.

### In the CLI

When you [create a workflow](creating-and-sharing-workflows.md#create-a-workflow) with the `/workflow` command, it gets saved to:

- Your working directory by default: `./workflow.yaml`
- Any path you specify: `/workflow /path/to/my-workflow.yaml`
- Local project workflows: `/workflow .biorouter/workflows/my-workflow.yaml`

> **Note.** The CLI saves workflows as `.yaml` files. While the CLI can run workflows in `.json` format, it does not provide an option to save workflows as JSON.

## Importing workflows

Workflow import is only available in Biorouter Desktop.

Import a workflow from a workflow file:

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` in the sidebar.
3. Click `Import workflow`.
4. Drop a `.yaml` or `.json` file on the dialog ("Drop a .yaml or .json file here"), or click the dialog to choose a file. Biorouter saves a copy of the workflow to your Workflow Library at once.

> **Warning.** Biorouter Desktop accepts `.yaml`, `.yml`, and `.json` files, but **the CLI only supports `.yaml` and `.json`**. For full compatibility across both interfaces, avoid `.yml` extensions.

All workflow formats follow the same [schema structure](workflow-schema-reference.md#core-workflow-schema).

## Finding available workflows

### In Desktop

**Access the Workflow Library:**

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows` to view your Workflow Library.
3. Browse your available workflows, which show:
   - Workflow title and description
   - Last modified date
   - Whether they're stored globally or locally

> **Note.** The Desktop Workflow Library displays all workflows you've explicitly saved or imported. It doesn't automatically discover workflow files from your filesystem like the CLI does.

### In the CLI

Use the `biorouter workflow list` command to find all available workflows from multiple sources:

```bash
# List all available workflows
biorouter workflow list

# Show detailed information including titles and full paths
biorouter workflow list --verbose

# Output in JSON format for automation
biorouter workflow list --format json
```

#### Workflow discovery process

Biorouter lists workflows from the following locations:

1. **Custom paths**: Directories specified in the [`BIOROUTER_WORKFLOW_PATH`](../configuration/environment-variables.md#workflow-configuration) environment variable
2. **Global workflow library**: `~/.config/biorouter/workflows/` (or equivalent on your OS)
3. **Local project workflows**: `./.biorouter/workflows/`
4. **GitHub repository**: If the [`BIOROUTER_WORKFLOW_GITHUB_REPO`](../configuration/environment-variables.md#workflow-configuration) environment variable is configured

Each of these is a directory that exists to hold workflows, so listing reads every `*.yaml` and `*.json` in it.

> **Note.** The working directory *root* is not one of them. It used to be, which meant `biorouter workflow list` opened and parsed every top-level `.yaml`/`.json` in whatever directory the session pointed at — package manifests, lockfiles, other tools' configuration. Set [`BIOROUTER_WORKFLOW_SCAN_WORKING_DIR=1`](../configuration/environment-variables.md#workflow-configuration) to list workflows kept loose in a project root, or move them into `.biorouter/workflows/`.

#### Running a workflow kept in the working directory

Discovery and lookup are separate. Naming a workflow — `biorouter run --recipe my-workflow`, or a `path:` in `sub_workflows` — searches the working directory too, in every case, because it opens exactly the file you named (`./my-workflow.yaml`, then `./my-workflow.json`) rather than enumerating the directory. A workflow the CLI's `/workflow` command saved to `./workflow.yaml` therefore still runs by name; it just does not appear in `biorouter workflow list` unless you opt in above.

#### Example output

*Default text format:*
```text
$ biorouter workflow list
Available workflows:
biorouter-self-test - A comprehensive meta-testing workflow - local: ./.biorouter/workflows/biorouter-self-test.yaml
hello-world - A sample workflow demonstrating basic usage - local: ~/.config/biorouter/workflows/hello-world.yaml
literature-scan - Summarize new PubMed papers on a topic - local: ~/.config/biorouter/workflows/literature-scan.yaml
```

*Verbose mode:*
```text
$ biorouter workflow list --verbose
Available workflows:
  biorouter-self-test - A comprehensive meta-testing workflow - local: ./.biorouter/workflows/biorouter-self-test.yaml
    Title: biorouter Self-Testing Integration Suite
    Path: ./.biorouter/workflows/biorouter-self-test.yaml
  hello-world - A sample workflow demonstrating basic usage - local: ~/.config/biorouter/workflows/hello-world.yaml
    Title: Hello World Workflow
    Path: /Users/username/.config/biorouter/workflows/hello-world.yaml
```

*JSON format for automation:*
```json
[
  {
    "name": "biorouter-self-test",
    "source": "Local",
    "path": "./.biorouter/workflows/biorouter-self-test.yaml",
    "title": "biorouter Self-Testing Integration Suite",
    "description": "A comprehensive meta-testing workflow"
  },
  {
    "name": "hello-world",
    "source": "GitHub",
    "path": "workflows/hello-world.yaml",
    "title": "Hello World Workflow",
    "description": "A sample workflow demonstrating basic usage"
  }
]
```

#### Configuring workflow sources

Add custom workflow directories:
```bash
biorouter workflow list
```

Configure GitHub workflow repository:
```bash
biorouter workflow list
```

See the [environment variables guide](../configuration/environment-variables.md#workflow-configuration) for more configuration options.

#### Manual directory browsing (advanced)

If you need to browse workflow directories manually:

```bash
# List workflows in default global location
ls ~/.config/biorouter/workflows/

# List workflows in current project
ls .biorouter/workflows/

# Search for all workflow files
find . -name "*.yaml" -path "*/workflows/*" -o -name "*.json" -path "*/workflows/*"
```

> **Tip.** The `biorouter workflow list` command is the recommended way to find workflows as it automatically searches all configured sources and provides consistent formatting.

## Using saved workflows

### In Desktop

1. Click the sidebar button in the top-left to open the sidebar.
2. Click `Workflows`.
3. Find your workflow in the Workflow Library.
4. Choose one of the following:
   - Click `Use` to run it immediately.
   - Click `Preview` to see the workflow details first, then click **Load Workflow** to run it.

### In the CLI

Once you've located your workflow file, [run the workflow](creating-and-sharing-workflows.md#run-a-workflow) or [open it in Biorouter Desktop](../cli/command-reference.md#workflow).

> **Tip.** The CLI can run workflows saved from Biorouter Desktop without any conversion. Both CLI-created and Desktop-saved workflows work with all workflow commands.

## Related documentation

- [Workflows](README.md) — the workflow file format and a summary of where Biorouter looks for workflow files.
- [Creating and sharing workflows](creating-and-sharing-workflows.md) — how the files described here get created, edited, and shared in the first place.
- [Workflow schema reference](workflow-schema-reference.md#core-workflow-schema) — the schema every stored workflow must satisfy.
- [Environment variables](../configuration/environment-variables.md#workflow-configuration) — `BIOROUTER_WORKFLOW_PATH` and `BIOROUTER_WORKFLOW_GITHUB_REPO`, which extend the search path.
- [biorouter CLI command reference](../cli/command-reference.md#workflow) — every `workflow` subcommand, including `list`, `open`, and `validate`.
