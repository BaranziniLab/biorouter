# How to Use AI with BioRouter
_A practical guide for contributing to BioRouter using AI coding assistants_

BioRouter benefits from thoughtful AI-assisted development, but contributors must maintain high standards for code quality, security, and collaboration. Whether you use BioRouter itself, GitHub Copilot, Cursor, Claude, or other AI tools, this guide will help you contribute effectively.

---

## Core Principles

- **Human Oversight**: You are accountable for all code you submit. Never commit code you don’t understand or can’t maintain.  
- **Quality Standards**: AI code must meet the same standards as human written code—tests, docs, and patterns included.  
- **Transparency**: Be open about significant AI usage in PRs and explain how you validated it.  

---

## Best Practices

**✅ Recommended Uses**  

- Generating boilerplate code and common patterns  
- Creating comprehensive test suites  
- Writing documentation and comments  
- Refactoring existing code for clarity  
- Generating utility functions and helpers  
- Explaining existing code patterns  

**❌ Avoid AI For**  

- Complex business logic without thorough review  
- Security critical authentication/authorization code  
- Code you don’t fully understand  
- Large architectural changes  
- Database migrations or schema changes  

**Workflow Tips**  

- Start small and validate often. Build, lint, and test incrementally  
- Study existing patterns before generating new code  
- Always ask: "Is this secure? Does it follow project patterns? What edge cases need testing?"

**Security Considerations**  

- Extra review required for MCP servers, network code, file system ops, user input, and credential handling
- Never expose secrets in prompts
- Sanitize inputs/outputs and follow BioRouter's security patterns  

---

## Testing & Review

Before submitting AI assisted code, confirm that:  
- You understand every line  
- Docs are updated and accurate  
- Code follows existing patterns  
- The checks below pass locally (happy path + error cases)
- No commit in your branch carries a `Co-Authored-By:` trailer naming an AI tool. The required `no-ai-coauthor` check rejects the branch until you rewrite the message, and several assistants add the trailer automatically

```bash
just check-everything               # fmt, clippy, UI lint, OpenAPI schema, version,
                                    # brand, Biorouter Copilot naming, cross-compile drift,
                                    # BAAM registry. Run before pushing; the checks that
                                    # actually block a merge are test (ubuntu/macos/
                                    # windows), Unit tests (vitest) and no-ai-coauthor.
cargo test -p <crate>               # the crates you touched
cd ui/desktop && npm run test:run   # frontend; bare `npm test` is watch mode
                                    # and will never exit
just generate-openapi               # REQUIRED after any server-route change
```

**Always get human review** for: 

- Security sensitive code  
- Core architecture changes  
- Async/concurrency logic  
- MCP protocol implementations  
- Large refactors or anything you’re unsure about  

---

## Using BioRouter for BioRouter development

- Protect sensitive files with `.biorouterignore` (e.g., `.env*`, `*.key`, `target/`, `.git/`)
- Guide BioRouter with `.biorouterhints` (patterns, error handling, formatting, tests, docs)
- Use `/plan` to structure work, and choose a permission mode deliberately. There are four, switched with the `/mode` slash command (see [`docs/security/permission-modes.md`](docs/security/permission-modes.md)):
  - **Completely Autonomous** (`auto`) — modifies files, uses extensions and deletes without approval
  - **Manual Approval** (`approve`) — confirms before every tool or extension use
  - **Smart Approval** (`smart_approve`) — risk-based; auto-approves low-risk actions, flags the rest
  - **Chat Only** (`chat`) — conversation only, no tools and no file modification
- **Completely Autonomous is applied by default.** So the action that matters is *turning the mode down* for critical work — not turning autonomy up. Chat Only for reading and understanding, Smart Approval for most dev work, Manual Approval for security-sensitive or hard-to-undo areas.
- The safety nets are not mode settings and cannot be toggled: a small fixed set of actions prompts even in Completely Autonomous — writes or deletes under a protected system directory, your home directory itself, or a credential store, and reads or writes of a **global** memory category. Do not mistake those prompts for the mode failing to apply.

---

## Community & Collaboration

- In PRs, note significant AI use and how you validated results  
- Share prompting tips, patterns, and pitfalls  
- Be responsive to feedback and help improve this guide  

---

## Remember

AI is a powerful assistant, not a replacement for your judgment. Use it to speed up development; while keeping your brain engaged, your standards high, and BioRouter secure.

Questions? [Open an issue](https://github.com/BaranziniLab/biorouter/issues/new/choose) to talk more about responsible AI development. GitHub Discussions is not enabled on this repository.  

---

## Getting Started with AI Tools

### Quick Setup

**Using BioRouter (meta!):**
```bash
# Install BioRouter — download the installer for your platform from
# https://github.com/BaranziniLab/biorouter/releases/latest
# (macOS .dmg, Windows .zip, Linux .deb/.rpm), or build the CLI from source
# with `cargo build --release` (binary at target/release/biorouter).

# Navigate to your BioRouter clone
cd /path/to/biorouter

# Start BioRouter in the repo
biorouter
```

**Using GitHub Copilot:**
- Install the [GitHub Copilot extension](https://marketplace.visualstudio.com/items?itemName=GitHub.copilot) for VS Code
- Enable GitHub Copilot for Rust files in your settings
- Recommended: Also install [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) for better code intelligence

**Using Cursor:**
- Download [Cursor](https://cursor.sh/) (VS Code fork with built-in AI)
- Open the BioRouter repository
- Use Cmd/Ctrl+K for inline AI editing, Cmd/Ctrl+L for chat

**Using Claude or ChatGPT:**
- Copy relevant code sections into the chat interface
- Provide context about the BioRouter architecture (see below)
- Always test generated code locally before committing

### Rust-Specific Configuration

If you're new to Rust, configure your AI tool to help you learn:

**VS Code settings.json:**
```json
{
  "rust-analyzer.checkOnSave.command": "clippy",
  "github.copilot.enable": {
    "rust": true
  }
}
```

**Cursor Rules (.cursorrules in repo root):**
```
This is a Rust project using cargo workspaces.
- Follow existing error handling patterns using anyhow::Result
- Use async/await for I/O operations
- Follow the project's clippy lints (see clippy-baselines/)
- Run cargo fmt before committing
```

---

## Understanding BioRouter's Architecture

New to AI agents? Here are key questions to ask your AI tool:

### Essential Concepts

**"Explain the BioRouter crate structure"**
```
Ask: "I'm looking at the BioRouter repository. Can you explain the purpose of each crate
in the crates/ directory and how they relate to each other?"

Key insight: BioRouter uses a workspace with specialized crates:
- biorouter: Core agent logic
- biorouter-cli: Command-line interface
- biorouter-server: Backend for desktop app (biorouterd)
- biorouter-mcp: MCP server implementations
```

**"How does the MCP protocol work in BioRouter?"**
```
Ask: "What is the Model Context Protocol (MCP) and how does BioRouter implement it?
Show me an example from crates/biorouter-mcp/"

Key insight: MCP allows BioRouter to connect to external tools and data sources.
Each MCP server provides specific capabilities (developer tools, file access, etc.)
```

**"What's the agent execution flow?"**
```
Ask: "Walk me through what happens when a user sends a message to BioRouter.
Start from crates/biorouter-cli/src/main.rs"

Key insight: Message → Agent → Provider (LLM) → Tool execution → Response
```

### Navigating the Codebase with AI

**Finding the right file:**
```
# Use ripgrep with AI assistance
Ask: "I want to add a new shell command tool. Where should I look?"
AI might suggest: rg "shell" crates/biorouter-mcp/ -l

Then ask: "Explain the structure of crates/biorouter-mcp/src/developer/shell.rs"
```

**Understanding patterns:**
```
Ask: "Show me the pattern for implementing a new Provider in BioRouter"
Then: "What's the difference between streaming and non-streaming providers?"
```

---

## Practical Examples

### Example 1: Understanding How to Add a New MCP Tool

**Scenario:** You want to add a new tool to the developer MCP server.

**Step 1 - Explore existing tools:**
```bash
# Ask AI: "Show me the structure of an existing MCP tool"
# The developer server's modules sit directly under developer/ — there is no
# tools/ subdirectory: shell.rs, text_editor.rs, background.rs, jail.rs,
# lang.rs, paths.rs, undo_history.rs, rmcp_developer.rs, mod.rs, plus the
# analyze/, editor_models/, prompts/ and tests/ directories.
ls crates/biorouter-mcp/src/developer/

# Pick a simple one to study
# Ask AI: "Explain this tool implementation line by line"
cat crates/biorouter-mcp/src/developer/shell.rs
```

**Step 2 - Ask AI to draft your new tool:**
```
Prompt: "I want to add a new MCP tool called 'git_status' that runs git status 
and returns the output. Based on the pattern in shell.rs, draft the implementation."
```

**Step 3 - Validate with AI:**
```
Ask: "Review this code for:
1. Proper error handling using anyhow::Result
2. Security concerns (command injection, etc.)
3. Async/await patterns matching the codebase
4. Test coverage needs"
```

**Step 4 - Test locally:**
```bash
# Build and test
cargo build -p biorouter-mcp
cargo test -p biorouter-mcp

# Run clippy
./scripts/clippy-lint.sh
```

### Example 2: Fixing a Rust Compiler Error

**Scenario:** You're getting a lifetime error you don't understand.

**Step 1 - Copy the full error:**
```bash
cargo build 2>&1 | pbcopy  # macOS
cargo build 2>&1 | xclip    # Linux
```

**Step 2 - Ask AI with context:**
```
Prompt: "I'm getting this Rust compiler error in the BioRouter project:

[paste error]

Here's the relevant code:
[paste code section]

Explain what's wrong and how to fix it following Rust best practices."
```

**Step 3 - Understand the fix:**
```
Ask: "Explain why this fix works and what I should learn about Rust lifetimes"
```

**Step 4 - Apply and verify:**
```bash
# Apply the fix
# Then verify it compiles and tests pass
cargo build
cargo test
```

### Example 3: Adding a Feature to the CLI

**Scenario:** You want to add a new command-line flag to biorouter-cli.

**Step 1 - Find the CLI argument parsing:**
```bash
# Ask AI: "Where does biorouter-cli parse command line arguments?"
rg "clap" crates/biorouter-cli/src/ -l
```

**Step 2 - Study the pattern:**
```
Ask: "Explain how biorouter-cli uses clap for argument parsing.
Show me how existing flags are defined."
```

**Step 3 - Draft your addition:**
```
Prompt: "I want to add a --verbose flag that enables debug logging.
Based on the existing patterns in biorouter-cli, show me:
1. How to add the flag to the CLI args struct
2. How to pass it to the biorouter core
3. How to use it to control log levels"
```

**Step 4 - Implement with validation:**
```bash
# Make changes
# Build both crates
cargo build -p biorouter-cli -p biorouter

# Test the new flag
./target/debug/biorouter --verbose session

# Run tests
cargo test -p biorouter-cli
```