# Model selection across windows

> **What this is.** The rules that keep the composer's model chip — its name, gauge, cost line and "Private model, UCSF" padlock — equal to what the next turn will actually run on, in every window, and the decision about what a model switch changes.
> **Status:** Current. Shipped for provider-QA finding F3 (2026-09-10) on top of `main` at `7c96d796`.
> **Audience:** developers working on the desktop renderer's model selection, the composer, or privacy tiers.

A chat runs on one of two things, and the chip has to state the right one at the moment a
person acts on it. Provider QA measured the failure on 2026-09-10: with two windows open,
changing the model in window 1 left window 2's chip reading `gpt-5.5-2026-04-24 (Private
model, UCSF)`, and the chat window 2 started bound `claude_code` — a consumer subscription
with no BAA (business associate agreement). The privacy barrier held: the chat was classified
`public`. What failed was the label the user read before typing.

## The two facts a chip can state

| Fact | Where it lives | What it decides | Who states it |
|---|---|---|---|
| **A chat's binding** | the session row (`provider_name`, `model_config`), outranked by the pin a turn reports | what an existing chat's next turn runs on — Gate B rebinds from the row | the chip inside a chat that has one |
| **The app-wide selection** | `BIOROUTER_PROVIDER` / `BIOROUTER_MODEL` in `config.yaml` | what a **new** chat binds — `/agent/start` reads exactly these two keys (`configured_new_session_provider`) and accepts no provider of its own | the chip on Home and in a chat not yet started |

`privacy/pinnedModel.ts` (`chatBinding`) and `privacy/usePinnedModel.ts` choose between the
two for a chat. This document is about keeping each one *current* — before F3 the second was
read once, when `ModelAndProviderContext` mounted, and never again.

## What a model switch changes

The model switcher (`SwitchModelModal`) is reached from the chip, from Settings → Models and
from onboarding, and every one of them ends in `ModelAndProviderContext.changeModel`.

| Opened from | What the switch changes | What the dialog says |
|---|---|---|
| a chat that exists | **that chat only** — its session row, through `/agent/update_provider` | "Select a provider and model for this chat." plus an unticked **Also use for new chats** box |
| a chat, with the box ticked | that chat **and** the app-wide selection | the box's hint: new chats in every window will start on it |
| Home, a chat not yet started, Settings → Models, onboarding | the app-wide selection — the only thing there is to change | "Select the provider and model new chats start on, in every window. Existing chats keep their own model." |

The success toast names which of the three happened (`switchedModelMessage`), and the chip's
dropdown, where there is no chat, is headed **Model for new chats** with the line "New chats
in every window start on this model. Existing chats keep their own."

> **Why.** Until 2026-09-11 a switch made in a chat also rewrote the app-wide selection,
> silently. QA F bound Claude Code in one chat for one check, and the next chat it opened came
> up public. [`privacy-tiers.md`](../security/privacy-tiers.md) §14.3 **P4** had already asked
> for this coupling to be undone — "pick Versa once in a scratch chat privatises not one
> session but every session created afterwards" — with an explicit "also make this my default
> for new chats" control. The box is that control, and it starts unticked because a switch
> made inside a chat reads as a statement about that chat.

## How each window stays current

Every window of the app is its own renderer with its own `ModelAndProviderContext`, over one
daemon.

- **Every renderer write announces.** `changeModel`, the first-run seeding of the bundled
  default (`getFallbackModelAndProvider`), and `ConfigContext.upsert` / `remove` of either key
  call `announceAppModelSelection` (`utils/sessionBindingSync.ts`). The last one covers the
  writers that never pass through `changeModel`: the local and coding-agent onboarding cards,
  Lead/Worker settings and Settings' reset. Before F3 those did not update even their own
  window's chip.
- **The announcement is a nudge, not a payload.** It travels on the same `BroadcastChannel`
  as the per-chat binding (`biorouter:session-binding`), shaped `{ kind: 'app-model-selection' }`
  and carrying no provider and no model. Two windows' writes can be announced in the opposite
  order from the one they landed in, so every receiver re-reads the daemon and ends on the
  write that landed last — the one `/agent/start` will bind.
- **A window re-reads when it regains focus or becomes visible.** Nothing announces a write
  made outside the renderer: `biorouter configure` in a terminal, or a hand-edited
  `config.yaml`. The daemon's config cache is keyed on the file's stamp, so `/agent/start`
  binds such a write at once.
- **Reads are ticketed.** The mount read, every re-read and the window's own switch each take
  a ticket when issued, and one publishes only if nothing issued after it has already been
  published. The comparison is against what was last *applied*, never what was last
  *issued* — see [renderer testing traps](renderer-testing-traps.md). A re-read that comes
  back with no body (a 500, a daemon that is restarting) changes nothing on screen: a failed
  read is not evidence that nothing is configured.
- **A re-read never writes.** `syncAppModelSelection` is a pure read. Only the mount-time
  `refreshCurrentModelAndProvider` may seed the bundled default, so neither a focus event nor
  another window's announcement can write config.

## The last look before a new chat

Both composers that create a chat — Home (`Hub.tsx`) and a chat not yet started
(`BaseChat.tsx`) — call `useConfirmNewChatModel` immediately before `createSession`, ahead
of anything the send consumes. It re-reads the pair and compares it with what the chip
showed. On a mismatch it:

1. publishes the fresh pair, so the chip, gauge, cost and padlock change;
2. raises a **Message not sent** toast naming the new model, its provider and its tier in
   words ("New chats now start on claude-fable-5-1 (Claude Code, a public model), not
   gpt-5.5-2026-04-24, which this window was still showing…");
3. resolves `false`, and `ChatInput` puts the text back.

It exists for the one write no ear hears in time: a `biorouter configure` in the terminal docked
**inside** the window, which never takes the window's focus. It refuses only a known mismatch —
nothing named on screen yet, or a read that failed, both proceed as before. It is not a gate:
the daemon classifies the chat by what it binds, whatever this check does.

## What this does not cover

- **The pin still outranks the row.** An app-wide change touches neither a session row nor a
  turn-reported pin, and a chat's chip goes on naming its own binding. The cross-window suite
  pins this.
- **Lead/Worker's `(lead)` / `(worker)` suffix** is read by `ModelsBottomBar` when it mounts.
  The model name beside it is live; the suffix in a second window is not.
- **An unannounced write while the window keeps focus** leaves the chip stale until the next
  focus change or the next new-chat send, which re-reads before it creates anything. An
  existing chat is unaffected either way: it runs on its own row.
- **The window between the last look and `/agent/start`** — two loopback round trips — is not
  closed. Closing it needs `/agent/start` to accept an expected binding and refuse a
  mismatch, which is daemon work.
- **`/config/set_provider` is not atomic.** `set_config_provider` writes
  `BIOROUTER_PROVIDER` and then `BIOROUTER_MODEL` as two config writes; measured on
  2026-09-11, `config.yaml` held `versa_azure` beside `gpt-6-astra` for about 55 ms of a
  Codex → Versa switch. A re-read caused by an announcement never sees it, because the
  announcement follows the write; one caused by a focus change landing in the gap could, and
  the announcement right behind it corrects the chip. A `/agent/start` landing in the gap has
  no such second chance and would bind the mixed pair. Daemon work.

## Tests

| Suite | What it pins |
|---|---|
| `components/ModelAndProviderContext.crossWindow.test.tsx` | two providers, each rendering the real chip over one fake daemon: a second window's chip — model, provider and privacy — follows the nudge without a remount; it equals what the next `/agent/start` would bind after a switch in either direction; ordering (a slow older read, a failed newer read); focus and visibility re-reads; the send refusal; a per-chat switch leaving new chats alone; the pin outranking a stale row |
| `utils/sessionBindingSync.test.ts` | the nudge is synchronous locally, carries no values, crosses windows, and never mixes with a binding |
| `components/ConfigContext.test.tsx` | `upsert` / `remove` of the two keys announce once the write resolved; other keys and refused writes do not |
| `components/privacy/useConfirmNewChatModel.test.tsx` | when the last look refuses and when it must not; both composers call it before `createSession`, pinned at the source |
| `settings/models/subcomponents/SwitchModelModal.test.tsx` | the dialog's scope copy and the unticked box |
| `settings/models/bottom_bar/ModelsBottomBar.pinned.test.tsx` | the "Model for new chats" heading where there is no chat |

```bash
cd ui/desktop && npx vitest run src/components/ModelAndProviderContext* src/utils/sessionBindingSync*
```

## Checking it in the running app

Launch a sandboxed instance with the dev GUI launcher on a free CDP port, open a second window
with `window.electron.createChatWindow()` from the first window's DevTools, and read each
window's chip by its accessible name (`button[aria-label^="Current model:"]`).

1. Change the model from window 1's Home chip (or tick **Also use for new chats** in a chat).
   Window 2's chip must read the new model within two seconds.
2. Send from window 2's Home composer, then read what the turn really ran on. The store sits
   under the sandbox's `data/`, not beside `config/`:

   ```bash
   sqlite3 -readonly ~/biorouter-runs/<run>/data/sessions/sessions.db "select provider, model_id from token_events order by id desc limit 1"
   ```

3. Repeat in the private → public direction. At no point may window 2's chip read "Private
   model, UCSF" while its next turn goes to a public model.
4. To see the last look refuse, hand-edit the two keys in `~/biorouter-runs/<run>/config/config.yaml`
   and send from window 2 without giving it focus. The chip changes, the text stays in the
   composer, and no session is created. ⚠ The toast carries the app's own class
   (`TOAST_SURFACE_CLASS_NAME`), not `Toastify__toast`: watch `section.Toastify`, or an
   observer will report "no toast" for one that rendered.

Measured on 2026-09-11 in a sandboxed instance at load average ~120, with both chips logged
every 50 ms against one clock: window 2's chip followed window 1's switch in 310 ms
(Versa → Codex) and 390 ms (Codex → Versa), in both cases directly from one correct label to
the other. The two turns sent from window 2 recorded `codex / gpt-6-astra` and
`versa_azure / gpt-5.5-2026-04-24` in `token_events`, each equal to the chip at the moment of
sending. The last look refused both directions of an unannounced hand edit within 100 ms,
with no focus or visibility event involved.

## Related documentation

- [Privacy tiers](../security/privacy-tiers.md) — §14.3 P4 is the decoupling this ships; the ledger at the top says what else shipped.
- [Renderer testing traps](renderer-testing-traps.md) — why the ticket compares against the last applied read.
- [The provider catalog](provider-catalog.md) — the other surface that opens the model switcher, including onboarding.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) — how to put two windows in front of you for the runtime check.
