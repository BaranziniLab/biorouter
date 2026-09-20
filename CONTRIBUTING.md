# Contribution Guide

Biorouter is open source!

We welcome pull requests for general contributions! If you have a larger new feature or any questions on how to develop a fix, we recommend you open an issue before starting.

> [!TIP]
> Beyond code, check out [other ways to contribute](#other-ways-to-contribute)

--- 

## 🤖 Quick Responsible AI Tips

If you use Biorouter, GitHub Copilot, Claude, or other AI tools to help with your PRs:

**✅ Good Uses** 

- Boilerplate code and common patterns  
- Test generation  
- Docs and comments  
- Refactoring for clarity  
- Utility functions/helpers  

**❌ Avoid AI For** 

- Security-critical logic  
- Complex business rules you don’t understand  
- Large architectural or schema changes  

**Quality Checklist**  

- Understand every line of code you submit  
- All tests pass locally  
- Code follows Biorouter's patterns
- Document your changes  
- Ask for review if security or core code is involved  

**⛔ Enforced by CI**

- **Strip AI co-author trailers before you push.** The `no-ai-coauthor` check
  rejects any commit message containing a `Co-Authored-By:` line naming an AI tool
  (Anthropic, Claude, OpenAI, ChatGPT, Gemini, Copilot) — and several of these tools
  add that trailer by default. It is a *required* status check, so one such trailer
  anywhere in your branch blocks the merge until you rewrite the message.

👉 Full guide here: [Responsible AI-Assisted Coding Guide](./HOWTOAI.md)

---

## Prerequisites

Biorouter includes rust binaries alongside an electron app for the GUI.

**Rust.** [Install rust and cargo][rustup]. You do not need to pick a version:
`rust-toolchain.toml` pins the channel (currently `1.92`) and rustup selects it
automatically inside the repo.

**Node 24.** `ui/desktop/package.json` declares `"engines": { "node": "^24.0.0" }`.
The easiest way to get a matching Node is the hermit environment that ships with
the repo — `source bin/activate-hermit` (see [Getting Started](#getting-started))
puts Node 24 on your PATH, so it is the recommended path. If you would rather
manage it yourself, [install node and npm][nvm] through nvm and select 24.

A newer Node is not a safe substitute, and it fails in ways that look like
application bugs rather than version problems: under Node 26 `electron-forge
package` exits 0 without producing an `.app`, and the `macos-alias` / `appdmg`
native modules the macOS dmg maker needs only build under Node 24.

We provide a shortcut to standard commands using [just][just] in our `justfile`.

### Windows Subsystem for Linux

For WSL users, you might need to install `build-essential` and `libxcb` otherwise you might run into `cc` linking errors (cc stands for C Compiler).
Install them by running these commands:

```
sudo apt update                   # Refreshes package list (no installs yet)
sudo apt install build-essential  # build-essential is a package that installs all core tools
sudo apt install libxcb1-dev      # libxcb1-dev is the development package for the X C Binding (XCB) library on Linux
```

## Getting Started

### Rust

First, activate the hermit environment and compile biorouter:

```
source bin/activate-hermit
cargo build
```

When that completes, debug builds of the binaries are available, including the biorouter CLI:

```
./target/debug/biorouter --help
```

For first-time setup, run the configure command:

```
./target/debug/biorouter configure
```

Once a connection to an LLM provider is working, start a session:

```
./target/debug/biorouter session
```

These same commands can be recompiled and immediately run using `cargo run -p biorouter-cli` for iteration.
When making changes to the Rust code, test them on the CLI or run checks, tests, and the linter:

```
cargo check  # verify changes compile
cargo test  # run tests with changes
cargo fmt   # format code
./scripts/clippy-lint.sh # run the linter
```

### Node

To run the app:

```
just run-ui
```

This command builds a release build of Rust (equivalent to `cargo build -r`) and starts the Electron process.
The app opens a window and displays first-time setup. After completing setup, Biorouter is ready for use.

Make GUI changes in `ui/desktop`. When you do, run the frontend checks from that
directory:

```
npm run test:run    # vitest unit tests
npm run lint:check  # typecheck + eslint (zero warnings) + theme codegen + contrast + token mirrors
```

`npm run test:run` is **required to merge** — it is the `Unit tests (vitest)`
status check on `main`, and it runs on every pull request whether or not you
touched `ui/desktop`. `npm run lint:check` covers most of the CI `Static checks`
job; that job additionally runs the packaged-dependency isolation tests
(`node --test scripts/verify-packaged-dependencies.test.cjs scripts/npm-command.test.cjs scripts/prepare-native-dependencies.test.cjs`)
and reports every gate rather than stopping at the first failure. It runs on
every pull request but is *not* a required check, so it will not block a merge
on its own.

Rust changes are gated by the `test (ubuntu-latest)`, `test (macos-latest)` and
`test (windows-latest)` checks from `.github/workflows/rust.yml`, which run
`cargo test --workspace --lib --bins` on each OS. All three are required, so a
failure on Windows alone blocks the merge; there is no way to run only the OS you
develop on.

### Running every check at once

`just check-everything` is the single precommit entry point. It chains
`cargo fmt`, the clippy lint script, `npm run lint:check`, the OpenAPI schema
check, the version, brand, and cross-compile-drift consistency checks, the
Biorouter Copilot naming gate (which rejects the string "Computer Controller" anywhere
a person reads it), and the two BAAM registry gates.

### Regenerating the OpenAPI schema

The file `ui/desktop/openapi.json` is automatically generated during the build.
It is written by the `generate_schema` binary in `crates/biorouter-server`.
To update the spec without starting the UI, run:

```
just generate-openapi
```

This command regenerates `ui/desktop/openapi.json` and then runs the UI's
`generate-api` script to rebuild the TypeScript client from that spec.

API changes should be made in the Rust source under `crates/biorouter-server/src/`.

### Debugging

To debug the Biorouter server, run it from an IDE. The configuration will depend on the IDE. The command to run is:

```
export BIOROUTER_SERVER__SECRET_KEY=test
cargo run --package biorouter-server --bin biorouterd -- agent   # or: `just debug-server`
```

`just debug-server` is the recipe that sets `BIOROUTER_SERVER__SECRET_KEY=test`, so
it pairs with `just debug-ui` below (which sends `X-Secret-Key: test`). Plain
`just run-server` does **not** set the secret, and a UI started with `just debug-ui`
cannot talk to it.

The server listens on port `3000` by default; this can be changed by setting the
`BIOROUTER_PORT` environment variable.

Once the server is running, start a UI and connect it to the server by running:

```
just debug-ui
```

The UI connects to the server started in the IDE, allowing breakpoints
and stepping through the server code while interacting with the UI.

## Creating a fork

To fork the repository:

1. Go to https://github.com/BaranziniLab/biorouter and click "Fork" (top-right corner).
2. This creates https://github.com/<your-username>/Biorouter under your GitHub account.
3. Clone your fork (not the main repo):

```
git clone https://github.com/<your-username>/biorouter.git
cd biorouter
```

4. Add the main repository as upstream:

```
git remote add upstream https://github.com/BaranziniLab/biorouter.git
```

5. Create a branch in your fork for your changes:

```
git checkout -b my-feature-branch
```

6. Sync your fork with the main repo:

```
git fetch upstream

# Merge them into your local branch (e.g., 'main' or 'my-feature-branch')
git checkout main
git merge upstream/main
```

7. Push to your fork. Because you’re the owner of the fork, you have permission to push here.

```
git push origin my-feature-branch
```

8. Open a Pull Request from your branch on your fork to BaranziniLab/biorouter's main branch.

## Keeping Your Fork Up-to-Date

To ensure a smooth integration of your contributions, it's important that your fork is kept up-to-date with the main repository. This helps avoid conflicts and allows us to merge your pull requests more quickly. Here’s how you can sync your fork:

### Syncing Your Fork with the Main Repository

1. **Add the Main Repository as a Remote** (Skip if you have already set this up):

   ```bash
   git remote add upstream https://github.com/BaranziniLab/biorouter.git
   ```

2. **Fetch the Latest Changes from the Main Repository**:

   ```bash
   git fetch upstream
   ```

3. **Checkout Your Development Branch**:

   ```bash
   git checkout your-branch-name
   ```

4. **Merge Changes from the Main Branch into Your Branch**:

   ```bash
   git merge upstream/main
   ```

   Resolve any conflicts that arise and commit the changes.

5. **Push the Merged Changes to Your Fork**:

   ```bash
   git push origin your-branch-name
   ```

This process will help you keep your branch aligned with the ongoing changes in the main repository, minimizing integration issues when it comes time to merge!

### Before Submitting a Pull Request

Before you submit a pull request, please ensure your fork is synchronized as described above. This check ensures your changes are compatible with the latest in the main repository and streamlines the review process.

If you encounter any issues during this process or have any questions, please reach out by opening an issue [here][issues], and we'll be happy to help.

## Env Vars

You may want to make more frequent changes to your provider setup or similar to test things out
as a developer. You can use environment variables to change things on the fly without redoing
your configuration.

> [!TIP]
> At the moment, we are still updating some of the CLI configuration to make sure this is
> respected.

You can change the provider Biorouter points to via the `BIOROUTER_PROVIDER` env var. If you already
have a credential for that provider in your keychain from previously setting up, it should
reuse it. For things like automations or to test without doing official setup, you can also
set the relevant env vars for that provider. For example `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
or `DATABRICKS_HOST`. Refer to the provider details for more info on required keys.

### Isolating Test Environments

When testing changes or running multiple Biorouter configurations, use `BIOROUTER_PATH_ROOT` to isolate your data:

```bash
# Test with a clean environment
export BIOROUTER_PATH_ROOT="/tmp/biorouter-test"
./target/debug/biorouter session

# Or for a single command
BIOROUTER_PATH_ROOT="/tmp/biorouter-dev" cargo run -p biorouter-cli -- session
```

This creates isolated `config/`, `data/`, and `state/` directories under the specified path, preventing your test sessions from affecting your main Biorouter installation. See the [environment variables reference](docs/configuration/environment-variables.md) for more details.

## Conventional Commits

This project follows the [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) specification for PR titles. Conventional Commits make it easier to understand the history of a project and facilitate automation around versioning and changelog generation.

[issues]: https://github.com/BaranziniLab/biorouter/issues
[rustup]: https://doc.rust-lang.org/cargo/getting-started/installation.html
[nvm]: https://github.com/nvm-sh/nvm
[just]: https://github.com/casey/just?tab=readme-ov-file#installation

## Developer Certificate of Origin

We ask contributors to add a [Developer Certificate of Origin](https://en.wikipedia.org/wiki/Developer_Certificate_of_Origin) sign-off to their commits. This is a statement indicating that you are allowed to make the contribution and that the project has the right to distribute it under its license. When you are ready to commit, use the `--signoff` flag to attach the sign-off to your commit.

```
git commit --signoff ...
```

Sign-off is currently a request, not a gate: there is no DCO check among the
repository's workflows and it is not one of the required status checks, so a
missing sign-off will not block your pull request. Most of the existing history
predates the practice.

## Contributing workflows

Workflows are reusable, shareable YAML files that capture a Biorouter session so
others can run it. Documentation and examples live in
[`docs/workflows/`](docs/workflows/). To share one with the
community, open a submission using the
[`Submit a workflow`](.github/ISSUE_TEMPLATE/submit-workflow.yml) GitHub issue
template and paste your YAML into the form — we'll review it and add it to the
cookbook.

## Other Ways to Contribute

There are numerous ways to be an open source contributor and contribute to Biorouter. We're here to help you on your way! Here are some suggestions to get started. If you have any questions or need help, feel free to reach out to us by [opening an issue](https://github.com/BaranziniLab/biorouter/issues).

> [!NOTE]
> GitHub Discussions is not enabled on this repository. Issues are the public
> venue for questions, feedback, and proposals.

- **Stars on GitHub:** If you resonate with our project and find it valuable, consider starring Biorouter on GitHub! 🌟
- **Ask Questions:** Your questions not only help us improve but also benefit the community. If you have a question, don't hesitate to [open an issue](https://github.com/BaranziniLab/biorouter/issues/new/choose).
- **Give Feedback:** Have a feature you want to see or encounter an issue with Biorouter? [Click here to open an issue](https://github.com/BaranziniLab/biorouter/issues/new/choose).
- **Improve Documentation:** Good documentation is key to the success of any project. You can help improve the quality of our existing docs or add new pages.
- **Help Other Members:** See another community member stuck? Or a contributor blocked by a question you know the answer to? Reply to open issues or do a code review for others to help.
- **Showcase Your Work:** Working on a project or written a blog post recently? Share it with the community in an [issue](https://github.com/BaranziniLab/biorouter/issues/new/choose).
- **Spread the Word:** Help us reach more people by sharing Biorouter's project and website.
