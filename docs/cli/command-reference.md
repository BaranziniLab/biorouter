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
- [Extensions, skills, and knowledge](#extensions-skills-and-knowledge)
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
biorouter models set --provider openai --model gpt-6-sol

# Local models for Llama Server: the catalog, what is downloaded, and from where
biorouter models local list
biorouter models local list --format json

# Download a model ahead of time, then stop the server again
biorouter models local pull qwen3.8-27b

# Delete a downloaded model's Hugging Face files
biorouter models local rm gemma4-31b
```

`models local` works on the shared model store directly, so it needs no running app or daemon.
`local list` labels each downloaded model with the copy Llama Server would start from: the
Hugging Face cache first, then a GGUF pulled into Ollama. `local pull` resolves a name the way a
chat does, so a name from an earlier catalog (such as `gemma4` or `qwen3.5-4b`) downloads the
model it now runs, `qwen3.8-27b`, and says so; when the retired model's own Hugging Face
download is still on disk, it also says how to free it. `local rm` resolves a name exactly, so
`local rm gemma4` deletes the files that retired model left behind and never the default
model's. A model pulled through Ollama is removed with `ollama rm`. See the
[Llama Server model catalog](../providers/llama-server/model-catalog.md).

### version

Check the current biorouter version you have installed.

**Usage:**

```bash
biorouter --version
```

### doctor [options]

Check system prerequisites and the CLI install. It prints one status line per prerequisite (Git, uv, Python 3, Node.js, AWS CLI, `llama-server`, the Rust toolchain), says whether `biorouter` is on your PATH, and reports the local model sidecar. Its **Dictation** section prints the lines of `biorouter dictation status`, and `--format json` carries the same object as `dictation`; it never starts a dictation worker, and lists microphones without opening one. Run it first when an extension will not start or a package runner is missing.

**Options:**

- **`--format <FORMAT>`**: `text` (default) or `json`
- **`--no-update`**: Skip the networked self-update check
- **`--fix [<DEPENDENCY>]`**: Hand a failing prerequisite to Biorouter. This opens a session briefed with the dependency, this machine's environment and what to verify. Pass a name (`--fix uv`), or leave it bare to take the first missing required one

**Usage:**

```bash
biorouter doctor
biorouter doctor --fix uv
biorouter doctor --format json --no-update
```

### dictation

Local dictation: the speech engine built into `biorouter` and `biorouterd`, and the speech models it runs. No model comes with an install or an update; you install one with `dictation install`, from the Baranzini Lab's releases on GitHub, into your own data folder (`~/.local/share/biorouter/dictation/models/` on macOS and Linux, `%LOCALAPPDATA%\UCSF\biorouter\data\dictation\models\` on Windows, `<root>/data/dictation/models/` under `BIOROUTER_PATH_ROOT`). Every file is checked against a SHA-256 pin compiled into Biorouter. The commands work offline on the files they find, need no running app or daemon, and print every sentence from the message catalog in [dictation surfaces](../dictation/surfaces.md#message-catalog). Design: [engine and models](../dictation/engine-and-models.md).

A refusal that means nothing was attempted exits `2` with its sentence alone: an agent's shell (`BIOROUTER_TERMINAL` set), no terminal without `--yes`, `--system` without root or Administrator, your organization's managed policy, an unknown model id, a build with no engine, or a processor that cannot run it. A failed download, hash or verification exits `1`.

#### dictation status [options]

Show whether this computer can dictate with Biorouter's local engine, and why: whether this build carries the engine, whether the processor can run it (with the sentence that says what it lacks), the backend and thread count, the worker's state and memory, the installed speech models and where they are, the model folder and account, what your organization's managed policy allows, and the terminal's microphone (`Microphone:`; in `--format json`, `capture` with `available`, `reason`, `backend` and the number of `devices`). The microphone line comes from listing the input devices without opening one, so no permission prompt appears; `reason` is `no_audio_system` where no PulseAudio, PipeWire or ALSA device answers (a server, a container), `no_device` where the audio system has no input, `policy` when your organization turned live dictation off, and `not_compiled` in a build without it. Reading the status never starts a dictation worker. It always exits 0. `biorouter doctor` prints the same lines in its **Dictation** section.

**Options:**

- **`--format <FORMAT>`**: `text` (default) or `json`, a versioned object (`"schema": 1`) for scripts; later versions only add keys
- **`-v, --verbose`**: Also start a worker only to read its first message (its protocol, engine build and CPU features) and stop it again, or show why it could not start, with the last lines it wrote to stderr. Under a managed policy that turns dictation off, nothing is started and the policy's sentence is shown instead

**Usage:**

```bash
biorouter dictation status
biorouter dictation status --format json --verbose
```

#### dictation list [options]

List every speech model in the catalog with its size, languages and how well it suits this computer, whether it is installed and in which folder, which one is in use, and any retired or unrecognized file found in a model folder. Alias: `ls`.

**Options:**

- **`--format <FORMAT>`**: `text` (default) or `json`

#### dictation install [options] [MODEL]

Download a speech model and the voice detector it needs, check each file against its pin, and place them in your data folder. With no `MODEL` it installs the one recommended for this computer: `whisper-large-v3-turbo-q5_0` (574 MB) on Apple Silicon, `whisper-small.en-q8_0` (264 MB) elsewhere, each with `silero-vad-v6.2.0` (885 KB). It shows each file's size and SHA-256, the source and the target folder with its account, then asks once at a terminal. A download that stops (Ctrl-C, a lost connection, a killed process) keeps its partial file, and the next `install` resumes from it. On Apple Silicon it then prepares the model in the background (one load and a short decode, so the first dictation does not wait for Metal's shaders to compile) and returns without waiting. Alias: `pull`.

**Options:**

- **`--from <FILE|DIR>`**: Install files you already downloaded, for example on a computer with no network. Each file is identified by its SHA-256, whatever its name; `biorouter dictation urls` lists what to download
- **`--system`**: Install into the computer's shared folder, which every account reads (`/var/lib/biorouter/dictation/models` on Linux, `/Library/Application Support/Biorouter/dictation/models` on macOS, `%ProgramData%\Biorouter\dictation\models` on Windows). Needs root or Administrator, and refuses a path whose folders other accounts could change or that passes through a link. Nothing is prepared in the background, because each account's Metal cache is its own
- **`-y, --yes`**: Install without asking. Needed without a terminal

It is always refused in an agent's shell: a person installs speech models, from their own terminal. On Linux, when a `biorouterd` on this computer runs as another account (the packaged `biorouter` service user), `install` and `status` say that models you install are not visible to it and print the `sudo -u <service user> -H biorouter dictation install` line to run instead.

**Usage:**

```bash
biorouter dictation install
biorouter dictation install --yes whisper-small.en-q8_0
biorouter dictation install --from ~/Downloads/
sudo biorouter dictation install --system
```

#### dictation remove [options] `<MODEL>`

Remove an installed speech model from your folder (or the shared one with `--system`). Removing the last speech model there removes the voice detector too; the detector stays while another speech model in that folder needs it or is being installed. It refuses while another Biorouter process installs the same model, asks at a terminal, and is never refused by managed policy. Alias: `rm`.

**Options:**

- **`--system`**: Remove the copy in the computer's shared folder (needs root or Administrator)
- **`-y, --yes`**: Remove without asking. Needed without a terminal

#### dictation use `<MODEL>`

Choose the speech model dictation uses; it is saved as `BIOROUTER_DICTATION_MODEL` in your configuration. It refuses a model that is not installed, one your organization's policy leaves out, and an English only model while the dictation language is not English. Refused in an agent's shell.

#### dictation urls [MODEL]

Print the download address, size and SHA-256 of a model's files and of the voice detector it needs (with no `MODEL`, the one recommended for this computer), for a computer that cannot reach GitHub: download them elsewhere, copy them over, then run `biorouter dictation install --from <file|dir>`.

**Usage:**

```bash
biorouter dictation urls whisper-small.en-q8_0
```

#### dictation verify

Hash every installed model file in every folder again and compare it with its pin. A file in your own folder that does not match is deleted; one in another folder is reported. Exits 1 when any file failed.

#### dictation bench [options]

Measure how fast this computer transcribes 5.5 s of the bundled sample with the model in use, and record the real time factor, which decides whether a slow model is offered as suitable here.

**Options:**

- **`--format <FORMAT>`**: `text` (default) or `json`

#### dictation devices [options]

List the microphones `biorouter dictate` can use, with the system default marked, without opening any of them, so no permission prompt appears. Where there are none it says why, in the words `dictate` would use: over SSH it gives the routes that work from your laptop, in a container or a WSL without WSLg it says so, and on a server with no audio system it names the alternatives. Set `BIOROUTER_CLI_DICTATION_DEVICE` to a name from this list to dictate from that device instead of the default. Under a managed policy that turns live dictation off it exits `2` with the policy's sentence.

**Options:**

- **`--format <FORMAT>`**: `text` (default) or `json` (`"schema": 1`, with `available`, `reason`, `backend` and each device's `name` and `default`)

#### dictation test [options]

Record from the microphone for a few seconds and report what arrived: the device and its rate, how much audio came in against the time it took, how soon the first audio came, and the level. Nothing is transcribed and nothing is kept. It is the first run path on macOS: the first time it opens the microphone, macOS asks your terminal app (Terminal, iTerm2, Ghostty, VS Code) for permission, and a microphone that delivers only exact silence gets the sentence that says how to allow it (exit `3`). Like `dictate`, it runs only at your own terminal, never in an agent's shell. Enter stops early and Escape cancels (exit `4`).

**Options:**

- **`--seconds <N>`**: How long to record, 1 to 600 (default 5)

### dictate [options]

Dictate from this terminal's microphone with the local engine and your personal dictionary. The microphone opens at once; the model loads while you speak, and what you have finished saying is transcribed while you go on, so after the stop only your last few seconds are left to transcribe, however long you spoke. Press Enter to stop, or Escape (or Ctrl-C) to cancel. The text is then shown for review on the terminal: Enter uses it, `e` edits it, Escape discards it. Only after that does it go to standard output (or the clipboard), so `"$(biorouter dictate)"` and a pipe send nothing you have not seen. An Enter or `e` pressed in the first 0.3 s after the text appears does not count, so a held key, or a second press of the Enter that stopped the recording, cannot send it before you have read it. Nothing leaves the computer, and the recording is never written to disk.

It runs only for you, at your own terminal: in an agent's shell (`BIOROUTER_TERMINAL` set) it refuses with "Dictation records your microphone, so only you can start it, from your own terminal. Biorouter does not start it from an agent's shell.", and without a terminal it refuses and points at `biorouter transcribe`, both exit `2`. Where no microphone can be reached it says so before asking for any model: over SSH it gives the routes that work (`biorouter serve` and a tunnel, `biorouter dictate --copy` on your laptop and paste, or a recording copied over and transcribed), naming the port of a `biorouter serve` your own account already runs on that Linux computer (another account's service is never named), and on macOS over SSH it explains that macOS lets only the logged in desktop session use the microphone. A recording stops by itself at 10 minutes, with a warning 30 s before.

Only your public dictionary terms shape the text unless you ask otherwise, because the text may go anywhere: `--private-terms` includes your private terms for this one recording after a warning you answer `y` to, and only at your own terminal with a private model configured.

**Options:**

- **`--copy`**: Put the text on the system clipboard instead of standard output. On Windows it is kept out of the cloud clipboard; on Linux, where a copy lasts only while its owner runs, it waits up to 60 s for you to paste. With no clipboard reachable (no display), the text is printed instead and the exit code is `5`
- **`--no-history`**: With `--copy`, ask clipboard managers not to keep the text in their history. Under WSL the copy goes through `powershell.exe Set-Clipboard`, which cannot keep it out of Windows clipboard history, so `--copy --no-history` is refused there before the microphone opens (exit `2`); run without `--copy` to print the text, or without `--no-history` to copy it anyway
- **`--format <FORMAT>`**: `text` (default) or `json` (`"schema": 1`, as `transcribe` writes it, with `destination` and the latency in `timings`: `stop_to_text_ms`, the tail that was left to decode, and the segments decoded while you spoke; on a failure `error_code` and `message`)
- **`--seconds <N>`**: Stop after N seconds, 1 to 600
- **`--session <SESSION_ID>`**: The chat the text is for. Its private dictionary terms are used only when the chat is private, you run this at your own terminal, and a private model is configured
- **`--private-terms`**: Let your private dictionary terms shape this one recording, after the warning; refused, exit `2`, without a terminal, in an agent's shell, or with no private model configured
- **`--language <LANGUAGE>`**: The language spoken, a two letter code such as `en`, or `auto`. Default: the dictation language
- **`--model <MODEL>`**: A speech model id from `biorouter dictation list`. Default: the model in use
- **`--no-dictionary`**: Dictate without the personal dictionary
- **`--no-review`**, **`-y, --yes`**: Print or copy the text without showing it for review first

**Environment:** `BIOROUTER_CLI_DICTATION_DEVICE` chooses an input device by the name `biorouter dictation devices` prints. On Linux, `BIOROUTER_DICTATION_RECORDER` names a command that writes raw 16 bit, 16 kHz, mono samples to its standard output, for an audio setup Biorouter's own capture does not reach, for example `BIOROUTER_DICTATION_RECORDER="arecord -q -f S16_LE -r 16000 -c 1 -t raw"`; it is started only when you start a recording.

**Exit codes:** `0` the text was written; `1` the transcription failed ("Transcription failed."); `2` declined to start (no terminal, an agent's shell, no microphone reachable here, the device would not open, policy, no engine, the processor, no model, `--private-terms` not admitted, `--copy --no-history` under WSL); `3` nothing was heard: no speech, under half a second, or only exact silence, which on macOS usually means your terminal app is not allowed to use the microphone; `4` cancelled; `5` `--copy` found no clipboard and printed the text.

**Usage:**

```bash
biorouter dictate
biorouter dictate --copy
biorouter run -t "$(biorouter dictate)"
biorouter dictate | biorouter crew send <channel> --input -
biorouter dictate --format json --no-review
```

### transcribe [options] `<FILE|->`

Transcribe a recording on this computer with the local engine and your personal dictionary, and print the text. Nothing leaves the computer. It reads WAV (any rate, depth and channel count), AIFF, FLAC, MP3, M4A and MP4 audio (AAC), raw AAC (ADTS) and Ogg (Vorbis or FLAC), or `-` for standard input, so a phone's or a Mac's voice memo needs no conversion; Opus, WebM and Apple Lossless are refused with an `ffmpeg` line that converts the file to WAV first. A recording may be up to 2 hours long, and standard input up to 200 MB. The command runs in cron, scripts, Docker and an agent's shell alike; in an agent's shell it uses only your public dictionary terms and records no use of them.

**Options:**

- **`--format <FORMAT>`**: `text` (default), `json` (a versioned object, `"schema": 1`: the text as corrected and as decoded, each correction in `changes`, the timed `segments`, the model, the language, `audio_s`, `bias_scope` and the timings; on a failure `error_code` and `message`), `srt` or `vtt`
- **`-o, --output <FILE>`**: Write the transcript to `FILE` instead of standard output
- **`--language <LANGUAGE>`**: The language spoken, a two letter code such as `en`, or `auto` to let the model decide. Default: the dictation language, `en` unless changed. `--format json` reports `auto` as given, not the language the model found
- **`--model <MODEL>`**: A speech model id from `biorouter dictation list`. Default: the model in use
- **`--session <SESSION_ID>`**: The chat the transcript is for. Its private dictionary terms are used only when the chat is private, you run this at your own terminal, and a private model is configured; otherwise only public terms are used, with no sign of the difference
- **`--context-file <FILE>`**: Text the recording is about (at most 4 KB is read). It ranks the dictionary terms that steer the transcript and is never sent anywhere
- **`--no-dictionary`**: Transcribe without the personal dictionary

**Exit codes:** `0` the text was written; `1` the input could not be read or decoded, the output could not be written, or the engine failed; `2` refused before anything was attempted (no engine, the processor, policy, no model, an unknown or uninstalled model, a language the model cannot take); `3` no speech was heard, or the recording was under half a second; `4` stopped with Ctrl-C.

Each `transcribe` loads its own copy of the model, so running many at once is slower than running them one after another: eight at once took 16 s in all, about 2 s each, while the same eight run one after another take about 2.6 s (0.33 s each).

**Usage:**

```bash
biorouter transcribe memo.mp3
biorouter transcribe interview.wav --format srt -o interview.srt
ffmpeg -i memo.m4a -ar 16000 -ac 1 -f wav - | biorouter transcribe -
biorouter transcribe notes.flac --context-file abstract.txt --format json
```

### dictionary

Manage your personal dictionary: the words dictation should spell your way, such as UCSF or EPAS1. Dictation steers recognition toward these terms and corrects close misses to them. There is one dictionary per account, in `<config>/dictionary/`; the commands work on it directly under its lock, with no daemon, and a term one terminal adds is listed by another at once. Each command prints the folder and account it acts on to stderr, so standard output stays clean for `list` and `export`.

A term is public, used for dictation into any chat, or private (`--private`), used only for dictation into chats on private models. A term may hold up to 64 characters and six words, and cannot be a web or email address, a file path, digits alone or something that looks like a key.

Every change except `import` needs you at an interactive terminal (exit `2` otherwise), and every change, `import` included, is refused in an agent's shell. Without a terminal, `list` and `export` show public terms only, and give no count of the rest. A question answered with Escape or a blank changes nothing and exits 0. Design: [dictionary](../dictation/dictionary.md).

#### dictionary list [options]

List the terms: written form, status, tier, source, uses and id. Alias: `ls`.

**Options:**

- **`--format <FORMAT>`**: `text` (default), `json` or `csv`
- **`--status <STATUS>`**: `active`, `suggested` or `all` (default)

#### dictionary add [options] `<TERM>...`

Add one or more terms. A term already there is reported, not added twice, and a term with the same letters as one you have (`Epas1` beside `EPAS1`) is added with a note.

**Options:**

- **`--spoken <FORMS>`**: How you say the term, comma separated; may repeat. With one term at a time
- **`--private`**: Use the terms only when dictating into chats on private models

#### dictionary edit [options] [TERM]

Change a term's spelling or its spoken forms. A term that is both public and private is named by `--id`.

**Options:**

- **`--id <ID>`**: The entry's id
- **`--term <NEW>`**: The new spelling
- **`--spoken <FORMS>`**: How you say the term, comma separated; replaces the spoken forms

#### dictionary remove `<TERM|--id <ID>>`

Remove terms; `--id` may repeat. A removed term stays out of automatic suggestions until you add it again. Alias: `rm`.

#### dictionary review

Review the terms the agent or Biorouter suggested, and the agent's terms you have not reviewed: choose the ones to keep, then whether to dismiss the rest.

#### dictionary accept `<ID>...` and dictionary dismiss `<ID>...`

Accept or dismiss suggestions by id, as `list --status suggested` shows them. `dismiss` deletes the row an id names with its tombstone, so a dismissed term is not suggested again. Known difference, to be narrowed: today `dismiss` does this for any row you can list, a term you added included, while the Dictionary page dismisses suggestions only; a later release makes `dismiss` act on suggestions only and answer any other id as not found.

#### dictionary import [options] `<FILE|->`

Merge terms from a file, or from standard input with `-`; an import never deletes. It reads one term per line (`EPAS1 = epas one, e pass one` adds spoken forms, and `#` after a space or at the start of a line begins a comment), CSV or TSV with a `written,spoken_forms,note` header, or a Biorouter export. It is the one change that works without a terminal, so the sync below can pipe into it, and it prints `Imported: <n> new, <n> merged, <n> duplicate, <n> invalid.` with a line for each refused row.

**Options:**

- **`--format <FORMAT>`**: `txt`, `csv`, `tsv` or `json`. Default: from the file's extension; standard input needs it unless it is an export
- **`--dry-run`**: Print what would change, and change nothing

#### dictionary export [options] [FILE|-]

Write your active terms as JSON, to standard output by default. The file carries no ids, chat ids or usage.

**Options:**

- **`--include-private`**: Include private terms, after a warning that the file leaves Biorouter. Needs your own terminal and a private model configured on this computer; on macOS and Linux a file that holds private terms is made readable by you alone

To use one computer's terms on another, for example a server you reach over SSH:

```bash
biorouter dictionary export - | ssh <host> biorouter dictionary import -
```

#### dictionary publicize [TERM] and dictionary privatize [TERM]

Make a private term public, or a public term private, after you type the term to confirm. `publicize` first warns that the term may then appear when you dictate into chats on public models. `--id <ID>` names one row when the term is both public and private.

#### dictionary settings [options]

Show the dictionary's settings, or change them. Raising a setting asks for confirmation; lowering one does not.

**Options:**

- **`--agent-writes <MODE>`**: What the agent's terms do: wait for you (`suggest`), go in when you ask or agree (`on_request`, the default), or go in at once (`add`)
- **`--learn-from-chats <ON|OFF>`**: Also find terms in the messages you write, automatically, on this computer

#### dictionary clear [options]

Delete every term, after you type `delete <n>` to confirm. Only the terms you were shown are deleted, so a term another process adds meanwhile stays.

**Options:**

- **`--suggested-only`**: Delete only the suggestions

**Usage:**

```bash
biorouter dictionary add EPAS1 --spoken "epas one"
biorouter dictionary add --private KOVALIN7
biorouter dictionary list --format csv
biorouter dictionary import terms.txt --dry-run
biorouter dictionary export terms.json
```

### usage [options]

Report token and cost usage, per day or per model, month-to-date.

**Options:**

- **`--from <YYYY-MM-DD>`**: Start of the range, local time. Defaults to 30 days ago
- **`--to <YYYY-MM-DD>`**: End of the range, local time and inclusive. Defaults to today
- **`--by-model`**: Group by model instead of by day
- **`--json`**: Print machine-readable JSON

**Usage:**

```bash
biorouter usage
biorouter usage --by-model --from 2026-09-01
```

### setup-path

Install the `biorouter` command onto your PATH. Alias: `install-cli`.

**Usage:**

```bash
biorouter setup-path
```

### completion `<SHELL>`

Generate the shell autocompletion script. `<SHELL>` is one of `bash`, `elvish`, `fish`, `powershell` or `zsh`.

**Options:**

- **`--bin-name <BIN_NAME>`**: Generate the script for a different binary name. Default is `biorouter`

**Usage:**

```bash
biorouter completion zsh > ~/.biorouter-completion.zsh
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
- **`--max-turns <NUMBER>`**: Set the maximum number of turns allowed without user input (`0` means Unlimited, the default)

**Model Options:**

- **`--provider <PROVIDER>`**: Override `BIOROUTER_PROVIDER` for this run. Available providers include `openai`, `anthropic`, `google`, `ollama`, `llamacpp` and `databricks`
- **`--model <MODEL>`**: Override `BIOROUTER_MODEL` for this run. The model must be supported by the provider in use

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

It removes the rest of what was kept under the session's ID too: the chat's text in the search index that Chat Recall reads, its checkpoints (the records and the snapshots of its working directory), and any cross-institution data flow you accepted in it. Biorouter also no longer hands a removed session's ID to the next session it creates, so nothing still kept under that ID can be read as a newer chat's. (Before, removing the newest chat of the day and starting another gave the new chat the same ID.) Subagent runs the chat started are separate sessions and are kept; `session list --subagents` shows them, and you can remove them with `--session-id` or `--regex ... --subagents`.

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

**The user-action key, when the daemon has one.** A daemon started with a user-action key (its
SHA-256 digest piped to `biorouterd agent` on stdin) wants the raw key before it lets anyone stop or
steer a turn, and before it takes a message into a subagent's session. `send`, `attach` and `cancel`
never ask for it up front: each makes its request without the key, and only if the daemon refuses
it for want of one asks you for it once, without echo, and tries again. `attach` asks as it joins,
before it reads anything you type. A daemon started without a key — `biorouter serve`, or
`biorouterd agent` with nothing piped in — is never asked about one. Such a daemon refuses every
request to stop, steer or send to a subagent's session, and the command prints its reason. To supply the key
without a prompt, pass `--user-action-key-stdin` and pipe the raw key as the first line of stdin.
Never put it in an argument, an environment variable or a config file. See
[Workspace control](../agent-loop/workspace-control.md#as-subcommands-you-type) for how to start a
daemon with a key.

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
- **`--user-action-key-stdin`**: Read the raw user-action key from the first line of stdin instead of being asked for it when the daemon wants it. See [the user-action key](#live-session-commands-and-the-daemon)

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
- **`--user-action-key-stdin`**: Read the raw user-action key from the first line of stdin, before the lines you steer with, instead of being asked for it when the daemon wants it. See [the user-action key](#live-session-commands-and-the-daemon)

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

**Options:**

- **`--user-action-key-stdin`**: Read the raw user-action key from the first line of stdin instead of being asked for it when the daemon wants it. See [the user-action key](#live-session-commands-and-the-daemon)

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

### session declassify

Lower a session's privacy classification from private to public, so it may run on any model. The change is confirmed at the terminal and recorded in the classification ledger. It works by session id, including for sessions no listing shows: subagent runs, `--no-session` runs and terminal sessions.

**Usage:**

```bash
biorouter session declassify 20251108_7
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
- **`--max-turns <NUMBER>`**: Maximum number of turns allowed without user input (`0` means Unlimited, the default)
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
biorouter run --provider anthropic --model claude-sonnet-5 -t "initial prompt"

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

Run one of the MCP servers bundled with Biorouter as a subprocess speaking MCP over stdio. MCP is the Model Context Protocol, the standard biorouter extensions speak. `<name>` is not an arbitrary extension name: it must be one of the five bundled servers below, and any other value is refused with `Invalid command: <name>`.

- `autovisualiser` (Auto Visualiser)
- `computercontroller` (the config key whose display name is "Biorouter Copilot"; the key kept its original spelling so existing configs and command lines keep working)
- `webdocuments` (the config key whose display name is "Web & Documents")
- `developer` (Developer)
- `memory` (Memory)

**Usage:**

```bash
biorouter mcp developer
```

### acp

Run biorouter as an Agent Client Protocol (ACP) agent server over stdio. This enables biorouter to work with ACP-compatible clients like Zed.

ACP is an emerging protocol specification that standardizes communication between AI agents and client applications, making it easier for clients to integrate with various AI agents.

**Usage:**

```bash
biorouter acp
```

> **Note.** This command is automatically invoked by ACP-compatible clients and is not typically run directly by users. The client manages the lifecycle of the `biorouter acp` process.

## Extensions, skills, and knowledge

### extension

Install and manage extensions. Alias: `ext`.

**Subcommands:**

- **`install <PATH>`**: Install an extension from a `.brxt` bundle. `--env <KEY=VALUE>` sets a plain environment variable and `--secret <KEY=VALUE>` a secret one (repeatable). A secret passed this way is visible to `ps`, so prefer the interactive prompt, or `--secret-stdin` to read `KEY=VALUE` lines from stdin in an unattended run. `--no-enable` installs without enabling
- **`list`**: List configured extensions
- **`configure <NAME>`**: Configure an installed extension's credentials
- **`remove <NAME>`**: Remove a configured extension

**Usage:**

```bash
biorouter extension install ./SPOKEAgent.brxt
biorouter ext list
biorouter ext configure SPOKEAgent
```

### skill

Install and manage skills.

**Subcommands:**

- **`install <SOURCE>`**: Install a skill or skill package from a `.zip` or a repository URL. `--force` replaces one already installed; `--as <bundle|individual>` answers the question a source that could be either one package or several separate skills raises
- **`list`**: List installed skills and their enabled state
- **`enable <NAME>`** / **`disable <NAME>`**: Turn a skill on or off without removing it. `<NAME>` is a skill name, a bundle name, or a slug as `skill list` shows it
- **`remove <NAME>`**: Remove an installed skill

**Usage:**

```bash
biorouter skill install https://github.com/BaranziniLab/biorouter-skills
biorouter skill list
biorouter skill disable code-review
```

### knowledge

Manage personal knowledge bases. Alias: `kb`.

**Subcommands:**

- **`list`**: List knowledge bases
- **`active`**: Show or set the primary base, which is the write target when no `--kb` is given. `--set <ID>` makes a base primary (it must not be hidden), `--clear` clears it, `--inherit` drops this scope's preference, and `--session <ID>` acts on one chat's primary instead of the machine-wide one
- **`create <ID>`**: Create a base with a kebab-case id. `--name <NAME>` and `--color <HEX>` are optional
- **`ingest`**: Ingest a source into a base. One of `--url`, `--file` or `--text`, with an optional `--kb <ID>`, `--focus <HINTS>`, `--provider` and `--model`
- **`ingest-conversation`**: Digest chat history into a base
- **`lint`**: Lint a base
- **`query <QUESTION>`**: Ask a base a question. `--save` persists the answer as a knowledge page
- **`hide`** / **`unhide`**: Hide a base from the agent, or bring it back

**Usage:**

```bash
biorouter kb create ms-genetics --name "MS genetics"
biorouter kb active --set ms-genetics
biorouter kb ingest --url https://example.org/paper
biorouter kb query "what is known about HLA-DRB1*15:01?"
```

### apps

List, open, and serve the Biorouter apps built by Agent Drafter. Apps open in a real browser; there is no in-terminal rendering.

**Subcommands:**

- **`list`**: List installed Biorouter apps. `--json` prints them as JSON
- **`open <ID>`**: Open an app in your default browser. A daemon this command starts keeps running after it returns, so the app stays open
- **`serve <ID>`**: Serve an app in the foreground until `Ctrl-C`, then stop the daemon it started. When a daemon it can use is already running, it prints a link and returns instead

**Which daemon they use.** When this profile's shared daemon is running (the one the desktop app and `biorouter crew` use), both commands open the app there and start nothing. They reach it the way every `crew` command does, and send the browser to that daemon's own address. `serve` then prints the link and returns, because that daemon keeps running on its own. A shared daemon they cannot use, for example one an older Biorouter started, is passed over with a line saying why. A port you name in `BIOROUTER_PORT` keeps its meaning: with `BIOROUTER_PORT=8080` set, the shared daemon is used only if it listens on port 8080, and otherwise the commands work on port 8080 as described below. (`BIOROUTER_PORT=0` names no port.)

Otherwise they look for a `biorouterd` from the configured port (`BIOROUTER_PORT`, default 3000) up, and start one on the first free port. Only a server whose `/status` answers `ok` counts as a Biorouter daemon; a port held by anything else, such as a web development server, is passed over for the next one, up to six ports. A daemon only opens an app for a caller holding its secret key, so they use a running daemon only when they know its key: a daemon `apps open` or `apps serve` started (they record its key in a file only your account can read), or one whose key `BIOROUTER_SERVER__SECRET_KEY` names. Any other daemon on the configured port, such as one you started by hand, is refused with a message saying so; set `BIOROUTER_SERVER__SECRET_KEY` to its key, or set `BIOROUTER_PORT` to a free port so the command starts its own. On Windows, the daemon `open` leaves running has no console of its own, so pressing `Ctrl-C` at that prompt later, or closing the window, does not stop it.

**The link opens the app once.** The address is `http://127.0.0.1:<port>/apps/<id>/?t=<token>`. It works once and for five minutes: opening it gives that browser the app, and the plain `http://127.0.0.1:<port>/apps/<id>/` address keeps working in that browser until the daemon stops or the browser is closed. `open` never hands the link to a command line, where other accounts on the machine could read it and use it first. It writes the link into a page only your account can read and opens that page. It also prints the link, for a browser that cannot open the page (a snap packaged browser cannot read hidden folders).

For another link, run `apps open` again. While `apps serve` is running, run `apps open <ID>` (or a second `apps serve <ID>`) in another terminal; it mints a new link on the same daemon. A browser that follows a spent or expired link sees a page that says how to open the app again.

**Usage:**

```bash
biorouter apps list
biorouter apps open cohort-explorer

# Serve in the foreground; open the printed link in a browser
biorouter apps serve cohort-explorer

# Another link while that serve runs, from a second terminal
biorouter apps open cohort-explorer
```

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
- **`--token <TOKEN>`**: Use this access token instead of generating a fresh one. The flag stays on the command line while `serve` runs, where other accounts on the machine can read it, so `serve` prints a note saying so. To keep a token private, set `BIOROUTER_BROWSER_TOKEN` in the environment instead
- **`--no-token`**: Serve without an access token. Refused for a non-loopback bind, and cannot be combined with `--token`
- **`--web-dir <DIR>`**: Directory holding the built interface. Takes precedence over `BIOROUTER_SERVE_UI`; whichever of the two is used must contain an `index.html`, or `serve` refuses to start. Located automatically when neither is set
- **`--computer-use-approval`**: Interactively set a Biorouter Copilot approval key for the browser session. Requires a terminal
- **`--open`**: Open a browser once the server is ready. The browser is handed a page only your account can read, which carries the address, and never the address itself: the address holds the access token, and a browser's command line can be read by other accounts on the machine. The page is removed two minutes later, or when `serve` stops

**Usage:**

```bash
# Serve on this machine only, and open a browser
biorouter serve --open

# Serve on a different port
biorouter serve --port 9000

# Reach it from another machine on the network — a token is mandatory here
biorouter serve --host 0.0.0.0

# Reuse one address across restarts, for a bookmark or a service unit
BIOROUTER_BROWSER_TOKEN="$(openssl rand -hex 32)" biorouter serve --host 0.0.0.0
```

The printed URL carries an access token as `?t=<token>`, minted per launch and shown once. Opening it exchanges the token for a session cookie and answers a short page that moves on to the interface, so the token leaves the address bar. It is not used up, and opens the interface again for anyone who has it until the daemon stops. Use `Ctrl+C` to stop the server, or send `serve` `SIGTERM` (`kill <pid>`); either way it stops the daemon it started and frees the port.

> **Note.** A browser session cannot change its model or provider, deliberately — run `biorouter configure` to choose them **before** starting `serve`. [Reaching Biorouter from a browser](../deployment/browser-access.md) explains why, and covers the access token, remote access and troubleshooting.

### web

> **Deprecated.** Use [`serve`](#serve) instead. `web` serves a minimal standalone chat page rather than the Biorouter interface, and its default port collides with `biorouterd`'s. It is kept for now; new deployments should not use it.
>
> Since 2026-09-11 it lists no chats and returns no transcripts, and it opens a private chat only if it started that chat itself while running a private model. Continue any other private chat in the desktop app. [SD-13](../deployment/serve-decisions.md#sd-13--biorouter-web-serves-no-transcripts-and-opens-no-private-chat-it-did-not-start) records why.

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

### term

Run a Biorouter session tied to your terminal window. Each terminal keeps its own persistent session and resumes it automatically. This is the command that installs the `@biorouter` and `@g` aliases described below.

**Subcommands:**

- **`init <SHELL>`**: Print the shell initialization script to evaluate from your shell profile. `--default` sends anything you type that is not a command to biorouter
- **`run <PROMPT>...`**: Run a prompt in the terminal session. Multiple words need no quotes
- **`info`**: Print compact session info (token usage, model) for prompt integration, for example `●○○○○ sonnet`

**Usage:**

```bash
# Set up, once
echo 'eval "$(biorouter term init zsh)"' >> ~/.zshrc
source ~/.zshrc

biorouter term run list files in this directory
```

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

The desktop composer, terminal UI (TUI), and classic CLI share these agent commands:

| Command | Action |
| --- | --- |
| `/compact` (alias `/summarize`) | Summarize the current chat to reduce context length. |
| `/clear` | Clear the current chat history. |
| `/effort [quick\|normal\|deep]` | Show or change this chat's reasoning effort. |
| `/goal <condition>` or `/goal clear` | Work toward a condition, or clear the goal. |
| `/loop <interval> <prompt>` | Run a prompt repeatedly; use `/loop` for status and stop syntax. |
| `/schedule <schedule> <prompt>` | Schedule a recurring prompt; use `/schedule list` to inspect jobs. |
| `/bug [description]` | Report a Biorouter bug. The agent investigates the chat, the documentation and the source, then drafts an issue you approve before anything is filed. See [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md#asking-biorouter-to-report-the-bug). |
| `/<workflow-command> [arguments]` | Run a configured workflow shortcut. A missing or invalid workflow reports an error. |

Type `/` to browse commands in the desktop or TUI; the classic CLI uses Tab completion.
Workflow shortcuts cannot reuse built-in command names or resource-reference syntax.

Resource references select a capability, active extension, skill, or knowledge base for the
agent's next response. They are not navigation commands and do not grant permissions:

| Reference | Example |
| --- | --- |
| `/ext:<id>` | `/ext:developer Explain the current project` |
| `/ext(<name with spaces>)` | `/ext(Biorouter Copilot) Inspect the approved test app` |
| `/skill:<name>` or `/skill(<name with spaces>)` | `/skill(literature review) Summarize these papers` |
| `/kb:<id>` or `/kb(<id with spaces>)` | `/kb:project-notes Find the latest decision` |

The desktop picker inserts references as chips, keeping their exact identifiers and
current display names. If a custom extension uses a bundled capability name, rename
it in Settings → Extensions before referring to it explicitly; those ambiguous custom
entries are omitted from the picker. Biorouter Copilot replaces the former Computer Use
and legacy Computer Controller display names; the `computercontroller` identifier remains
compatible. `/extend` is not an extension command: it gives guidance to use `/ext:`
instead. Biorouter Copilot still requires its normal runtime, OS permissions, and per-task
approval.

The desktop also offers `/knowledge` as a Knowledge reference shortcut and `/diverge`
to continue an existing conversation in a new chat. The message action is also labelled
**Diverge**. Start a chat before using it.

In the desktop composer, `/bug` followed by a space becomes a **Report a bug** chip at the
start of the message. The chip is only how the composer draws it: the message is still
sent as `/bug <text>`.

The TUI and classic CLI additionally support `/help` (alias `/?`), `/exit` (alias `/quit`),
`/rename <name>`, and `/diverge [name]`. The following commands are **classic CLI only**;
start it with `BIOROUTER_CLI_CLASSIC=1 biorouter session`:

- `/builtin <names>` — add built-in extensions by name, comma-separated.
- `/extension <command>` — add a stdio extension, with optional `ENV=value` prefixes.
- `/mode <name>` — select `auto`, `approve`, `chat`, or `smart_approve` mode.
- `/plan [message]` and `/endplan` — enter or leave plan mode.
- `/workflow [file.yaml]` — export the current conversation as a workflow.
- `/t [light|dark|ansi]` — cycle or set the terminal theme.
- `/r` — toggle full, untruncated tool output.

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
