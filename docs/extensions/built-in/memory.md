# Memory capability

> **What this is.** User guide to the built-in Memory capability: enabling it, the trigger words that store, recall and forget memories, where memories live on disk, and a worked example teaching BioRouter a lab's analysis standards.
> **Status:** Current. `crates/biorouter-mcp/src/memory` ships in the product and the local/global model described here still matches. This page predates the Knowledge feature; see [Knowledge and memory](#knowledge-and-memory) for how the two relate.
> **Audience:** end users.

The Memory capability lets you teach BioRouter personalized information — commands, code snippets, preferences, configurations, lab conventions — that it can recall and apply later. Knowledge can be project-specific (**local**) or user-wide (**global**), so BioRouter remembers what matters to you across sessions.

## Configuration

1. Run the `configure` command:

   ```bash
   biorouter configure
   ```

2. Choose `Toggle Extensions`, then enable `memory`:

   ```text
   ┌   biorouter-configure
   │
   ◇  What would you like to configure?
   │  Toggle Extensions
   │
   ◆  Enable capabilities and extensions: (use "space" to toggle and "enter" to submit)
   │  ● memory
   └  Extension settings updated successfully
   ```

## Why use Memory

With the Memory capability you are not just storing static notes, you are teaching BioRouter how to assist you better. Imagine telling BioRouter:

> _learn everything about MCP servers and save it to memory._

Later, you can ask:

> _utilizing our MCP server knowledge help me build an MCP server._

BioRouter recalls everything you have saved, as long as you instruct it to remember. This makes results more consistent across sessions.

BioRouter loads all saved memories at the start of a session and includes them in every prompt sent to the LLM. For large or detailed instructions, store them in files and instruct BioRouter to reference those files:

> _Remember that if I ask for help writing JavaScript, I want you to refer to "/path/to/javascript_notes.txt" and follow the instructions in that file._

## Where memories are stored

Memories are plain files on your machine, organized by category. There are two scopes:

| Scope | Location |
|-------|----------|
| Global (user-wide) | `~/.config/biorouter/memory/` on macOS and Linux; `%APPDATA%\UCSF\biorouter\config\memory` on Windows |
| Local (project-specific) | `.biorouter/memory/` inside the working directory |

Because they are ordinary files, **you** can inspect, edit or delete them directly when you want to audit or reset what BioRouter has learned. You do not have to, though — the desktop app lists both stores and lets you prune them.

BioRouter itself cannot take that shortcut on the global store. Reading, changing or deleting a machine-wide memory is put to you for approval by category, and that approval would be worth little if the same files could just be opened by other means — so the global store is closed to everything but the memory tools:

- The file tools (`text_editor`, the Web & Documents cache) refuse any path inside it and point at the memory tool to use instead.
- Tool calls made from inside an `execute_code` script are refused: a script's calls are not shown to you one at a time, so there is nothing for you to approve. The model is told to make the memory call directly.
- The `/agent/call_tool` API is refused for the same reason.
- BioRouter's memory server run on its own — `biorouter mcp memory`, for another MCP client outside the app — serves the project-local store only. Nothing there can ask you, so nothing there reaches the machine-wide store.

The project-local store is unaffected by all of this: it lives under the directory you opened, so it never crosses into another session.

A stray file in the global store directory is ignored rather than fatal, and a category name is treated as a label — no control characters, and bounded in length — because those names are listed in every later session's system prompt.

A category name also has to be a *filename*, and on every platform rather than only the one you are using. It becomes `<name>.txt` on disk, so anything Windows refuses in a filename is refused everywhere: the characters `< > : " | ? *`, a name ending in a dot or a space, and the reserved device names (`con`, `prn`, `aux`, `nul`, `com0`-`com9`, `lpt0`-`lpt9`). The rule is not local to Windows because a memory store travels — a synced config directory, a machine you moved to — and a name one computer can write but another cannot open is a store that stops working when it arrives. Dots inside a name, spaces inside a name and non-ASCII are all still ordinary. `*` is on that list for a second reason as well: it is the "every category" argument on `retrieve_memories` and `remove_memory_category`, so it cannot also be one category's name without meaning two things at once.

One shape is refused rather than put to you: reading the *whole* global store in a single call. The reason is that the disclosure should be no larger than the task — an answer needs the memories bearing on your question, not every memory every conversation on this computer has ever saved — and there is always a narrower way to get it, because the category names are in the model's prompt and it can ask for one by name. Nothing becomes unreachable: every global memory is still readable, one approved category at a time. Deleting the whole store is *not* refused, because "forget everything" has no narrower substitute — expressing it one category at a time would be more confirmations for a single intention, and could not reach a category you did not know about.

## Seeing and deleting what was remembered

Open **Settings → Chat → Memory**. It lists both stores separately: **Global**, shared by every chat on this computer, and **This project**, which only chats opened in that project can read. Each store shows how many memories it holds, and the info icon beside its name shows who can read it and the folder it lives in. Expand a category to read its contents.

This matters most for the global store. A conversation asking to read a global memory category has to be approved by you first, and that approval card names the category; this section is where you find out what is in it before deciding. The card names that folder, which is the one the info icon beside **Global** shows, so you know where to look.

What each memory shows is what the store actually records, and no more:

| Shown | Where it comes from |
|-------|---------------------|
| Category name | The category file's name |
| Scope, and the store's absolute path | Which of the two directories the file is in |
| Tags | The memory's own `#` tag line, when the model attached one |
| Category size and "Updated" date, in the tooltip on the category's name | The category file's size and modification time on disk |

A memory is a line in a flat text file, so nothing records **when an individual memory was written, which conversation wrote it, or which model**. The date shown is per category (the last time anything was appended to that file), not per memory, and the section words it that way rather than dating each row with a timestamp the file cannot support.

Two delete controls sit alongside the listing: one on each memory, one on each category. A trash icon appears at the end of a row when you point at it or move the keyboard onto it, and is always shown on a touch screen. Both ask first, and the confirmation says what is about to be lost — the number of memories going, who could read them, and that it cannot be undone. Deleting the last memory in a category deletes the category too, so its name stops being offered to future sessions. Nothing is recoverable afterwards; BioRouter keeps no copy.

## What a delete does, exactly

Deleting is precise and it is final, in both directions.

- **"Forget X" removes one memory.** The model has to name a memory in full; a partial or approximate phrase deletes nothing and comes back as an error rather than a false report of success. Asking to forget "black" will not also remove "we use black for formatting".
- **Deleting the last memory in a category removes the category**, so its name stops appearing in future sessions' prompts.
- **"Forget everything" removes the memories, not the folder.** Anything else you keep in the store directory — your own notes, a subfolder — is left alone.
- **A delete you confirmed applies to the list you were shown.** If a conversation writes to a category while a confirmation is open, the delete is refused and asks you to reload rather than acting on a list that has moved on. Reloading and clicking again works normally.

## Available tools

| Tool | Description |
|------|-------------|
| `remember_memory` | Store a memory in a category, with optional tags, in the local or global scope |
| `retrieve_memories` | Read back stored memories |
| `remove_memory_category` | Delete an entire category of memories |
| `remove_specific_memory` | Delete one stored memory, named in full |

## Trigger words and when to use them

BioRouter recognizes certain words as signals to store, retrieve or remove memory. These are **soft heuristics that guide the model**, not a parsed command grammar — a phrasing that is not in this table can still trigger a memory operation, and a phrasing that is in it will not always do so. State your intent plainly and the model will pick the right tool.

| **Trigger words**   | **When to use** |
|---------------------|----------------|
| remember            | Store useful info for later use |
| forget           | Remove a stored memory |
| memory           | General memory-related actions |
| save             | Save a command, config, or preference |
| remove memory    | Delete specific stored data |
| clear memory     | Wipe all stored memories |
| search memory    | Find previously stored data |
| find memory      | Locate specific saved information |

## Knowledge and memory

The Memory capability is best for short, durable facts and preferences that should ride along in every prompt. For larger bodies of structured knowledge — ingested documents, cross-referenced pages, a browsable graph — BioRouter's Knowledge feature is the better fit, and it stores its content separately under `~/.config/biorouter/knowledge/`. See [knowledge ingestion formats](../../knowledge-base/ingestion-format-roadmap.md) for what Knowledge can absorb.

## Example usage

This example makes BioRouter a knowledgeable research assistant by teaching it your lab's data-analysis standards. BioRouter stores the standards and recalls them when needed, so you do not have to repeat your project's requirements on every new task.

> **Tip.** If you frequently work with analysis standards or other structured knowledge, BioRouter may offer to save them to memory for you when it detects a pattern in your workflow.

To follow along, either open a new session in BioRouter Desktop, or start one from a terminal:

```bash
biorouter session
```

### Step 1: Teach BioRouter your analysis standards

#### BioRouter prompt #1

```text
biorouter, learn our cohort data-analysis standards:

- All cohorts must be defined with explicit OMOP concept IDs
- Continuous variables are reported as mean ± SD; categorical as n (%)
- Multiple-comparison correction uses Benjamini-Hochberg FDR
- Patient identifiers must be de-identified before analysis
- Every analysis needs a reproducible script and a methods summary

Remember this so you can help me follow these practices.
```

#### BioRouter output

```text
I've learned your analysis standards. I'll help enforce these requirements when working on cohort analyses.

Storing in:
    - Category: analysis_standards
    - Tags: #cohort #omop #statistics #reproducibility
    - Scope: global (these are general standards)

I can now help with:
    - Cohort definition with OMOP concepts
    - Descriptive and inferential statistics
    - Multiple-comparison correction
    - De-identification and methods reporting
```

### Step 2: Use stored knowledge to run a new cohort analysis

#### BioRouter prompt #2

```text
Help me compare baseline characteristics between the treatment and control cohorts
```

#### BioRouter output

```text
I see you need a baseline comparison. Based on our cohort data-analysis standards that I have stored in memory, I'll ensure it meets all our requirements.

[.. Checking stored analysis standards ...]

I've followed all our required elements:

✅ Cohorts defined via OMOP concept IDs
✅ Mean ± SD for continuous, n (%) for categorical
✅ Benjamini-Hochberg FDR correction
✅ De-identified patient data
✅ Reproducible script + methods summary

Here's your compliant analysis script:

# baseline_comparison.py
import pandas as pd
from scipy import stats
from statsmodels.stats.multitest import multipletests

treatment = load_cohort(concept_ids=[201826])   # OMOP concept
control = load_cohort(concept_ids=[4193704])

# Continuous: mean ± SD, two-sample t-test
# Categorical: n (%), chi-square test
pvals = run_baseline_tests(treatment, control)

# Benjamini-Hochberg FDR correction per our standard
_, qvals, _, _ = multipletests(pvals, method="fdr_bh")

Would you like me to run the full comparison and generate the Table 1 summary?
```

## Related documentation

- [Chat Recall capability](chat-recall.md) — the complementary way to recover past context, by searching old sessions rather than storing facts.
- [Knowledge ingestion formats](../../knowledge-base/ingestion-format-roadmap.md) — the Knowledge feature, which covers larger bodies of structured knowledge.
- [Extensions and skills guide](../extensions-and-skills-guide.md) — how to enable, disable and configure extensions generally.
- [Context engineering](../../agent-loop/context-engineering.md) — how memories, skills and other injected context share the window.
