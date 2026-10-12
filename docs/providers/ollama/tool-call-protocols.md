# Tool-call protocols

> **What this is.** How Ollama turns a local model's output into a tool call, what 12 models
> actually returned on the wire, which local models work well as a Biorouter agent and which
> to avoid, and the guards on recovering a tool call a model wrote as text.
> **Status:** Current. Measured on 2026-10-10 against Ollama 0.40.2 on an Apple M4 Max shared
> with other workloads. The short protocol probes used temperature 0; the later live agent runs
> used the model's default sampling and their task results varied. Timings describe that shared
> machine, including queueing and memory contention. The handling section was checked against
> `crates/biorouter/src/providers/ollama.rs`, `llamacpp.rs`, `formats/ollama.rs` and
> `formats/text_tool_calls.rs` on the same date.
> **Audience:** developers working on the Ollama provider, and anyone choosing a local model.

A model does not call a tool. It writes text in its own tool-call format, and the server,
here Ollama, is supposed to recognise that text and return it as a structured
`message.tool_calls` entry. Each model family writes a different format, and Ollama picks a
different way to read it per model. When that reading fails, the call either leaks into the
reply as text (the "model writes JSON instead of doing the task" symptom) or disappears and
leaves an empty reply. So the same Biorouter request gets a clean tool call from one model,
JSON text from a second, and nothing at all from a third.

The probes behind this page sent six scenarios to each model on three endpoints: native
`/api/chat` streamed and not streamed (both with `num_ctx` and `truncate: false`), and
`/v1/chat/completions` streamed. The scenarios were one shell call, two parallel calls, a
call with object and array arguments, a question needing no tool ("what is 17 + 25"), a call
whose arguments include `name` and `description` (Biorouter's create-app tool), and an answer
after a tool result. These are short prompts, not Biorouter's full ~31,000-token agent prompt,
so a model that passes them may still fail as an agent; the two recommended models were tested
with the full prompt in live agent runs.

## The five ways Ollama handles a model

Ollama chooses one source for each model's prompt template and tool-call parser, in this order
of precedence (`selectedTemplateSource` in Ollama's `server/images.go`), and that choice
decides how robust tool calling is:

| Path | How Ollama selects it | How a call is read | Models probed |
| --- | --- | --- | --- |
| **Built-in renderer and parser** | The Modelfile names a `RENDERER` and/or `PARSER` written in Go for that model family. | Code that knows the family's exact format. The most reliable path. | `qwen3.8:27b` (renderer `qwen3.8`, parser `qwen3.5`), `gemma4:31b` (`gemma4`), `glm-4.7-flash` (`glm-4.7`), `lfm2:24b` (`lfm2`), `ministral-3:8b` (parser `ministral`, Go template), `olmo-3`, `nemotron-3-nano` |
| **Harmony** | The model uses OpenAI's Harmony format. | Harmony's channel parser. | `gpt-oss:20b` |
| **Go template** | The Modelfile carries a Go template that renders tools. | A generic parser that watches the output for the template's tool tag, then reads JSON after it. Fragile: anything the model writes differently leaks or vanishes. | `llama3.1:8b` (tag `{`), `qwen2.5-coder:7b` and `qwen3:1.7b` (`<tool_call>`), `mistral-small3.2:24b` (`[TOOL_CALLS]`), `granite4:tiny-h` (`<tool_call>`), `phi4-mini:3.8b` (`<\|tool_call\|>[`), `command-r7b:7b` |
| **GGUF chat template** | No Go template; the GGUF file's own Jinja template runs in Ollama's llama-server runner. | llama.cpp's template-based parsing. | `deepseek-r1:8b` |
| **No tool support** | Nothing renders tools, so `/api/show` lists no `tools` capability. | Not at all: a request with tools is refused with HTTP 400 `... does not support tools`. | `gemma3:4b` |

You can see which path a model takes with `ollama show --modelfile <model>`: a `RENDERER` or
`PARSER` line means the first path. `/api/show`'s `capabilities` says whether the model can
take tools and think at all.

## What a structured call looks like

These held for every model that produced structured calls:

- **Native `/api/chat`.** Each call is
  `{"id": "call_<8 characters>", "function": {"index": N, "name": ..., "arguments": {...}}}`.
  There is no `type`, `index` sits inside `function`, and `arguments` is always a JSON object.
- **Calls are never streamed in fragments.** Each call arrives complete in one chunk. Most
  models send N calls as N separate chunks and then a `done` chunk with no calls; `gpt-oss`
  sends its single call inside the `done` chunk itself.
- **`done_reason` is always `stop`**, even when the turn made calls, and `length` only at the
  output budget. A turn's finish reason has to be inferred from whether calls arrived.
- **`/v1/chat/completions`.** Each call arrives complete in one delta, with `arguments` as a
  compact JSON string, followed by `finish_reason: "tool_calls"` and a usage chunk.
- **Argument key order is not stable** (Go maps). Compare parsed objects, never strings.
- **Call ids differ between streamed and non-streamed replies** and between retries; they are
  not a way to deduplicate.

## Per-model results

"Structured calls" counts the four call-expecting scenarios on the two native endpoints, so
8 of 8 is every expected call arriving as a structured call. The two recommended models were
measured differently: in live Biorouter agent runs with the full prompt.

| Model | Path | Structured calls | What goes wrong |
| --- | --- | --- | --- |
| **qwen3.8:27b** | Built-in | Every call, in live agent runs | Nothing observed. Correct answers on a clinical-notes count and a write-and-run script, on both the MLX and GGUF builds. |
| **gemma4:31b** | Built-in | Every call, in live agent runs | Correct on both tasks through Ollama. On the script task through Llama Server, with code execution on, it wrote code calling `text_editor.write(...)`, which that mode does not provide, and repeated it until the loop guard stopped the turn (two runs). Its calls were structured; the code inside them was wrong. |
| glm-4.7-flash | Built-in | 5 of 5 with its default thinking; 8 of 8 with `think: false` | Nothing wrong with calls. Slow to load (124-560 s cold on a shared GPU). |
| ministral-3:8b | Built-in parser | 8 of 8 | A call to a tool that does not exist kills the whole turn: HTTP 500 `tool '<name>' not found`, or, after text has streamed, a final error line with no `done` chunk. On `/v1` the same error is silently dropped. |
| gpt-oss:20b | Harmony | 6 of 8 complete (8 of 8 made at least one call) | Only one call per reply, so parallel work takes extra round trips. Reasoning cannot be turned off (`think: false` still returns some). Ignores enum constraints. |
| llama3.1:8b | Go template | 8 of 8 | Its template drops every tool definition once the last message is a tool result, so a multi-step task gets no call and a **fabricated** answer (0 of 3 on a chained task). Argument drift: `"10"` for an integer, a nested array sent as a JSON string. |
| granite4:tiny-h | Go template | 6 of 8 complete (8 of 8 made one) | A tool name not in the list gives an empty reply. Text after a call is silently dropped. Asked for a tool it lacked, it ran `mail` through the shell. |
| lfm2:24b | Built-in | 6 of 8 | Refuses to make two calls at once and invents the results instead. A shell command containing both quote kinds gives an empty reply. Trained on a 32,768-token window, too small to hold Biorouter's prompt with room to work. |
| qwen3:1.7b | Go template | 4 of 8 | Half its calls vanish after thinking (reasoning, then empty content and no call); picks the wrong tool on `/v1`. Too small for agent work. |
| mistral-small3.2:24b | Go template | 6 of 8 | **Silent empty replies** whenever a call's arguments contain a `name` key, which includes Biorouter's create-app tool, or name a tool that does not exist. Its template shows the tool list only next to the last user message, so it is tool-blind after the first tool result. |
| qwen2.5-coder:7b | Go template | **0 of 8** | Writes every call as bare JSON text, `{"name": ..., "arguments": ...}`, and parallel calls as a JSON array. This is the reported symptom, on every call. |
| phi4-mini:3.8b | Go template | **0 of 8** | 0 structured calls in 63 requests: refusals (in English, Chinese and Russian), arrays with the tool name in the wrong field, a literal `<\|tool_call\|>` followed by prose. |
| command-r7b:7b | Go template | **0 of 8** | Its shipped template ends the prompt in a way that prevents calls; it answers "I don't have the tools available", and once claimed "I have created an app" with no call. |
| deepseek-r1:8b | GGUF template | **0 of 8** | Never calls a tool and **fabricates tool output**, such as an invented `ls` listing. `think: false` is ignored, and a stray `</think><think>` leaks into the answer. |
| gemma3:4b | No tools | HTTP 400 | Cannot take tools at all; Biorouter's model list hides it. |

## Which models to use

**Use one of the two recommended models**, `qwen3.8:27b` (the default) or `gemma4:31b`. Both run
on the built-in path, both handled Biorouter's full ~31,000-token prompt with structured calls
throughout, and both need about 32 GiB of GPU-addressable memory.

Other models that made reliable structured calls in the probes, best first: `glm-4.7-flash`
with thinking off, `ministral-3:8b`, `gpt-oss:20b`, then `llama3.1:8b` and `granite4:tiny-h`
for single-step work only. Later full-prompt agent runs answered the clinical-notes count correctly
with `gpt-oss:20b` and `mistral-small3.2:24b`; the latter also hit two 600-second prefill timeouts
under contention. `llama3.1:8b` and `granite4:tiny-h` did not finish that task in Code Execution
mode because they misused its module API. `glm-4.7-flash` and `ministral-3:8b` were tested only
with the short protocol probes. A successful count is evidence for that run, not the same
coverage as the recommended models.

**Avoid these as an agent**, and expect the failure named:

- `qwen2.5-coder:7b`: never makes a structured call through Ollama. Text recovery let a lean
  Developer + Todo run finish the clinical-notes count, but a later run gave the wrong answer
  and Code Execution runs did not finish. Recovering its call format does not fix its task errors.
- `mistral-small3.2:24b`: blank replies on common tool calls, and tool-blind after the first
  result.
- `phi4-mini:3.8b`: no structured calls at all.
- `deepseek-r1:8b`: invents tool output, which is unsafe in an agent loop.
- `command-r7b:7b`: its template prevents calls, and it can claim work it did not do.
- `gemma3:4b`: no tool support.
- `qwen3:1.7b`: too small; calls vanish or go to the wrong tool.
- `lfm2:24b`: its 32,768-token window is too small for Biorouter's prompt.

## Text a model wrote instead of a call

Ollama strips every model control token except the leading tool tag. So formats that live in
special tokens never reach Biorouter as text: Ollama either parses them or drops them. These
are the leaked formats that actually appeared in `message.content`:

| Format | Example | Seen in |
| --- | --- | --- |
| L1: the whole reply is one call object | `{"name": "developer__shell", "arguments": {"command": "ls /tmp"}}`, compact or pretty-printed | every `qwen2.5-coder:7b` call; `gemma3:4b` through the tool shim |
| L2: the whole reply is an array of call objects | `[{"name": ..., "arguments": ...}, {"name": ..., "arguments": ...}]` | `qwen2.5-coder:7b` parallel calls |
| L3: prose, then a call object using `parameters` | `I will list the files ... {"name": "developer__shell", "parameters": {...}}` | `llama3.1:8b`, only with Biorouter's small-model prompt, which asked it to explain before acting |
| L4: prose, then the call in a fenced `json` code block | `Okay, let's do this. First, I'll run the ls /tmp command.` followed by the fenced object | `gemma3:4b` through the tool shim |
| L5: an array with the tool name in `type` | `[{"type": "get_weather", "function": {"name": "get_weather", "arguments": {...}}}]` | `phi4-mini:3.8b` |
| L6: prose, then a backticked call | `` `developer__shell({"command": "uname -a"})` `` | `granite4:tiny-h`, only when asked to narrate first |
| L7: a bare `name{json}` | `knowledge_base_search{"query": "beta.csv"}` | `mistral-small3.2:24b`, only on a request that offered no tools |
| L8: an L1, L2 or L5 payload wrapped whole in `<tool_call>…</tool_call>`, closing tag optional | `<tool_call>{"name": "developer__shell", "arguments": {...}}</tool_call>` | `qwen2.5-coder:7b` in live agent runs, after a tool result |

The same model also wrapped calls in the template's RESULT tag, `<tool_response>…</tool_response>`.
That tag is where a template puts tool output, and `qwen2.5-coder:7b` wrote it both to echo a
real result and to invent one (both seen live on 2026-10-10), so a payload inside it is never
recovered as a call.

Text that looks like a call but is not one, and must stay text:

- An answer that is JSON: `{"sum": 42, "ok": true}` (`gpt-oss:20b`, asked for a JSON answer).
- A plain answer: `42`.
- A fenced object naming something that is not a tool, with an invented `output` field
  (`deepseek-r1:8b`'s fabricated `ls`).
- An object whose `name` is an argument value: `{"name": "Cohort Explorer", "description": ...}`.
- A JavaScript snippet that calls a tool by name, after a literal `<|tool_call|>`.
- A stated intention with no call: "Next step: I will use the shell tool ...".

Never seen as text on the wire, so there is no point parsing them: Hermes `<tool_call>` blocks
anywhere but around the whole reply (L8), `[TOOL_CALLS]`, Harmony channels, `<|python_tag|>`, pythonic `[f(a="x")]` lists, Qwen3-Coder
`<function=...>` blocks, Gemma 4's `<|tool_call>call:`, GLM `<arg_key>`, Cohere
`<|START_ACTION|>` and DeepSeek's tool-call tokens. They appeared only in raw generations and
rendered prompts. The probes sent single requests; L8 appeared only later, in multi-step agent
runs.

## Guards on recovering a call from text

A call recovered from text is a guess about what the model meant, and running a guessed
`developer__shell` call is worse than showing the text. Recovery runs in two places:

- **The local providers**, Ollama and Llama Server, run it on every reply, always.
- **The tool shim** (`BIOROUTER_TOOLSHIM`, off unless you turn it on) runs it on any provider's
  reply, before it asks its interpreter model, because in that mode text is the tool protocol
  by design.

Wherever it runs, recovery is fenced in: every one of these must hold, or the reply stays text
exactly as the model wrote it.

1. **The request offered tools.** The tool names come from that request, never from a global
   list. Outside the tool shim, only a local provider's replies are read.
2. **No structured call in the reply.** Recovered calls are never mixed with real ones.
3. **A complete reply.** On the local providers the reply ended normally, not at its output
   budget, which would leave half a JSON object. The tool shim does not see the finish reason;
   there a call cut short fails to parse as JSON and stays text.
4. **Whole-reply shape.** The reply is exactly a call object, a sequence of them, or an array of
   them (L1, L2, L5), optionally in a `json` code fence, or one of them wrapped whole in
   `<tool_call>` tags (L8). Only the tool shim also accepts prose
   first (L3, L4): at most 400 characters of prose containing no `{`, no code fence and no
   JSON array, then one block, bare or fenced, which may hold one call object or an array or
   sequence of them.
5. **Exact tool names.** Every name is byte-for-byte a tool from the request: no case folding,
   no prefix stripping (`shell` is not `developer__shell`), no fuzzy match.
6. **Required fields and basic types.** Arguments must be an object. Recovery checks required
   fields, directly declared JSON types, nested properties and array items, and
   `additionalProperties: false`, to a depth of 16. It does not resolve `$ref` or evaluate
   `enum`, `anyOf`, `oneOf`, or other JSON Schema constraints; those remain for the tool's
   normal argument handling. Recovery is not a full JSON Schema validator.
7. **All or nothing.** One bad call in an array rejects the whole array.
8. **A closed key set.** A call object has only `name` with `arguments` or `parameters`, plus
   optional `id` and `type`. An object with any other key is an answer, not a call.
9. **Marked and logged.** A recovered call gets a fresh id and is logged with the model and the
   format.
10. **No shortcut past approval.** A recovered call goes through the same permission checks as
    any other; being recovered never approves it.

On the local providers, while a reply that starts with `{`, `[`, a code fence or `<tool_call>`
is streaming, its text is held back rather than shown, so the user does not watch raw JSON being typed. The
guards then decide at the end of the reply whether it becomes calls or is shown as text. The
tool shim reads a reply only after it has streamed, so there the text stays visible and any
recovered calls follow it.

## Empty replies and error lines

- **An empty reply** (normal end, no text, no call, but tokens generated) usually means Ollama
  read a call it could not parse and dropped it. It cannot be recovered from the reply. Seen
  with `mistral-small3.2:24b` (nine times), `granite4:tiny-h`, `lfm2:24b`, `qwen3:1.7b` and a
  crashed `phi4-mini:3.8b` runner (an HTTP 200 with an empty body).
- **`tool '<name>' not found`** (HTTP 500, or a final error line after text) is the model
  calling a tool that does not exist, not the server failing. It must not be mistaken for
  "model not found", which also contains "not found".
- **`... does not support tools`** (HTTP 400) is a capability answer: retrying cannot help.
- **A stream that ends without its `done` chunk** and without an error is a broken connection,
  not an empty answer. Biorouter reports it and runs none of the calls from it.
- **An error object can arrive four ways:** as the body of a non-200 response; as the last
  line of a 200 stream; with llama.cpp's JSON error encoded inside the string (context
  overflows); or as plain text (`prediction aborted, token repeat limit reached`).

## How Biorouter handles each outcome

The Ollama provider (`crates/biorouter/src/providers/ollama.rs`, with the wire format in
`formats/ollama.rs` and recovery in `formats/text_tool_calls.rs`) handles each outcome above as
follows.

| Outcome | What Biorouter does |
| --- | --- |
| Structured calls | Collects every call of the reply, across chunks, and holds them until the reply's usage shows the prompt was not cut; only then do they run. A missing id is minted, and arguments may be an object or a JSON string. Arguments are normalised against the tool's schema: Cohere's `{tool_name, parameters}` wrapper is unwrapped, an object or array sent as a JSON string is decoded, and `"10"` or `"true"` becomes a number or boolean where the schema asks for one. An enum violation is left for the tool to report, so the model sees the error and can correct it. |
| The whole reply is a call written as text (L1, L2, L5, L8) | Recovered under the guards above. While the reply starts like a call (`{`, `[`, a code fence or `<tool_call>`) its text is held back; thinking still streams. A recovered call carries a `recovered_from_text` marker and its format, and one log line names the model, the format and the count. If the guards reject it, the held text is shown unchanged. |
| The whole reply is a call that recovery declined (a name without its extension prefix, such as `execute_code` for `code_execution__execute_code`; arguments that do not fit; the `<tool_response>` tag around it) | Asked once, using the turn's one retry. The note says text in a reply is never run as a tool call, and asks the model to make the call as a call if it intended one, otherwise to answer in text. It never orders a call. A reply of several call objects, or an array of them, is asked about once, naming every tool, when each object names an offered tool (diagnostics 20261006_2: `llama3.1:8b` wrote `workspace_read_conversation` and `workspace_read_panel` in one reply). No retry when a name stands for no offered tool (a JSON answer shaped like a call), or when the same call already appears in the conversation: made by an earlier turn, or quoted in a tool result or an earlier assistant reply. Best-effort: if the retry cannot be sent, fails, overflows, or comes back empty, the turn ends with the reply already shown. |
| The whole reply is only calls written as text, and none ran (a tool the chat does not offer directly, such as Developer's `shell` in a chat with Code Execution on; an extension's name; a retry that repeated the call) | Shown as it came, then one line for the person only: "Nothing ran: this reply is a call to 'shell' written as text, not a tool call Biorouter can run." with a note that a larger model or fewer extensions may help. The agent adds the line after the reply, so it appears for every provider; it is stored with the chat and never sent to the model. Nothing in the reply is run. |
| Prose, then a call (L3, L4) | Left as text, and never acted on: a reply that quotes a call (a summary, a relayed tool result, a requested example) cannot be told from one that meant it. Only the tool shim (`BIOROUTER_TOOLSHIM`), where text is the tool protocol by design, recovers this form, on whichever provider it is turned on for, and it does so before asking its interpreter model. |
| An empty reply to a request with tools | Retried once when its tokens may be a call Ollama swallowed: with no reasoning, more than 8 output tokens; with reasoning, always, because measured reasoning ran 2.4-4.7 characters per token and `qwen3:1.7b`'s swallowed call fell inside that band. The retry carries a note listing the tools the model can call. The error then says only what the numbers show. "Could not parse as a tool call" appears when the output exceeds a ceiling for the reasoning (characters/2 + 16 tokens) by more than 8. Otherwise the error says the model "returned reasoning only (N output tokens)" and may also have written an unparsable call. The advice names only the recommended model not in use. A reply cut at its output budget, or one with no reasoning and at most 8 tokens, fails at once. |
| `tool '<name>' not found` | Text already shown is kept, and the model is asked once more with a note that the tool does not exist and the list of tools it can call. If that fails too, the error names the tool. It is never reported as a missing model. |
| A model without tool support | Refused before the request is sent, with a message saying the model cannot call tools. Such models are also left out of the model list. A server that answers the 400 anyway, on the native path or the `/v1` fallback, gets the same message. |
| Think tags inside the answer | Moved to thinking only in a reply that opened with a think block or a tag's tail (deepseek-r1's `>`), including its stray `</think><think>`; in any other reply the tags are the answer's own text and are kept. |
| A stream that ends without `done` | An error; none of its calls run. |

A turn gets at most one corrective retry, shared by the unknown-tool, empty-reply and
whole-reply-call cases. The retries run on the native `/api/chat` path only; a server on the
`/v1` fallback gets the errors without one. When a retry follows text the user has already
read, the retry's first text starts a new paragraph rather than running on from the last
sentence.

The bundled Llama Server provider (`llamacpp.rs`) runs its streamed and blocking replies through
the same recovery: whole-reply calls only (L1, L2, L5, L8), the same guards, the same hold-back
and the same argument normalisation. llama-server's own parser handles the structured path
there, so the three retries above are Ollama's alone.

Two more measures come from the probes and act on the request rather than the reply:

- Every tool result names its tool and follows the order of the calls, because several Go
  templates pair results with calls by position and `gpt-oss` renders a nameless result as an
  empty function name. For templates that drop an assistant turn's calls when the turn also has
  text, the calls are sent without the text.
- A model whose template shows the tool list only next to the latest user message
  (`llama3.1:8b`, `mistral-small3.2:24b`, recognised from the template `/api/show` returns)
  sees no tool definition after the first tool result. For such a model Biorouter appends a
  short tool roster to the system prompt: one line per tool with its name, its parameters (`?`
  marks an optional one) and the first sentence of its description, at most 64 tools. The
  roster goes on every request for that model, not only after a tool result, so the prompt
  prefix stays the same and Ollama keeps it cached. Biorouter also logs one warning per model,
  because a multi-step task may still stop early or be answered without the tools.

## Server effects a user will notice

- **Cold loads dominate.** 60 to 1,125 seconds were observed while other 27-31B models held the
  GPU; most of that was queueing, not the model's own load time.
- **No model streams a call's arguments**, so a tool turn shows nothing until the whole call is
  ready: 3 to 8 seconds of silence for a 68-token figure call.
- **Without `num_ctx`** (the old `/v1` path) a 128 GiB machine requested 655k to 1.3M tokens of
  cache, which produced hangs of 9 to 15 minutes, empty replies, and a stall that blocked every
  other client of the same Ollama. See
  [context window and settings](context-window-and-settings.md).

## Related documentation

- [Context window and settings](context-window-and-settings.md): how a request is built, and
  every setting that changes it.
- [Ollama provider](README.md): the folder index.
- [Llama Server provider](../llama-server/README.md): the same two supported models, served by
  the bundled llama.cpp sidecar.
- [Environment variables](../../configuration/environment-variables.md): `BIOROUTER_TOOLSHIM`
  and the other tool-handling settings.
