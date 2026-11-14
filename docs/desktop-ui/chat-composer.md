# The chat composer

> **What this is.** How the chat composer is laid out, what its `+` popover holds, how a chat's built-in capabilities and contexts are switched for that chat alone, and how the composer moves.
> **Status:** Current (owner direction, 2026-11-13).
> **Audience:** anyone changing `ChatInput.tsx`, the `+` popover, the footer line or the tool lists.

## One line

The composer is one card and one footer line, the same on Home, in a chat, in a split pane and in a subagent's tab (all of them render `ChatInput`).

The card holds, from the top: the message queue, any notice (no model, a usage limit, an image the model cannot read, dictation's line), the chips that go with the message, and then ONE row:

| Slot | What it is |
|---|---|
| `+` | Attach files, mention, commands, and the chat's tool lists (below). A decision chat has none. |
| The text | Grows upward with what is typed, then scrolls. |
| Microphone | Dictation, when it is available. |
| Send or Stop | Always the row's last child. |

The controls keep to the text's last line, so a long draft never moves them. The row is `.br-composer-row` in `composer/composer.css`; each control sits in a 32px slot, centred, so the 28px microphone and the 32px buttons share one centre line with a single line of text.

Under the card, the footer line (`bottom_menu/ComposerFooter.tsx`): the working folder on the left; the model, the context ring and the chat's cost (where its usage limit is set) on the right. On Home only the folder and the model show. The model chip opens its menu above the card, never over the draft (`ModelsBottomBar`'s `menuClearOf`). The folder, the ring and the cost open footer cards: a hover peeks, a click pins ([shared controls](shared-controls.md), "Hover and click").

## The `+` popover

`composer/ComposerPlusMenu.tsx` replaced two controls: the old `+` menu and the Tools chip. It opens one popover above the `+`, in the Tools popover's style (`.br-picker-popover`), with one list at a time and a segmented control on the bottom edge:

- **Add**: Attach files, Mention (`@`), Commands (`/`) and, in an ordinary chat, Decision models and monitors. A click on `+` always opens here.
- **Extensions**, **Skills**, **Knowledge**: the lists that used to be under the Tools chip, with their search field, scope line, bulk action and switches.

The segmented control never moves (R-25): it is the popover's last child, Radix pins a `side="top"` popover by its bottom edge, and collision handling is off, so a longer list grows the popover upward. The control's thumb slides on the spring like every segmented control. The counts the Tools chip used to carry (extensions, skills and knowledge bases on in this chat) are now the `+`'s tooltip and accessible name.

## Built-in capabilities and contexts, per chat

The shipped capabilities (Developer, Biorouter Copilot, Web & Documents, Auto Visualiser, Memory, Knowledge, Agent Drafter and the rest of Settings > Chat > Capabilities) and the shipped Contexts (Word, PowerPoint, Excel, PDF, Biorouter overview, the Develop Biorouter guides, Knowledge) follow Settings for every new chat. Each one can also be switched for one chat:

- The Extensions list ends with a folded **Built-in capabilities** group. A switch there adds or removes that capability for this chat only, through the same per-chat path as an installed extension, and the change is saved with the chat.
- The Skills list ends with a folded **Built-in contexts** group. A switch there writes the chat's own skill override (`workspace_skills/v1`).

Both groups are collapsed by default (owner, 2026-11-13), so the lists show only what the person added. A group opens on a click, or while a search matches one of its rows, and folds again every time the popover closes (`useBuiltInDisclosure` in `bottom_menu/PickerPanel.tsx`, pinned by `PickerPanel.test.tsx`).

Two capabilities have no per-chat switch, because the daemon would undo it: Workspace Control (in Completely Autonomous mode the agent re-adds a delegation copy every turn) and Crew (a `/crew` grant or a Crew task supplies its tools, and only Revoke ends that). A capability or Context with nothing to switch reads "Not available in this app".

Per chat beats Settings in both directions: a chat can turn on a Context that Settings has off, and turn off one that Settings has on (`compose_state` in `crates/biorouter/src/agents/skill_catalog.rs`).

A subagent the chat starts inherits both: the chat's capabilities as they are, and a copy of its Context choices on the subagent's own chat, so the subagent's `+` popover shows them. The chat's agent can change either for one subagent; see [Subagents](../agent-loop/subagents.md#extension-control). On Home, where there is no chat yet, a capability switch shapes the next chat and a Context switch is the Settings switch itself. Built-ins never join Enable all or Disable all, and they are not counted in the `+`'s tooltip.

## Motion

All of it is transform and opacity on the motion tokens (`styles/motion.ts`), and none of it plays under reduced motion.

| Moment | What moves |
|---|---|
| Empty to written | Send turns accent and its arrow settles in on the spring (`data-armed`). |
| Sending | The sent text lifts 8px and fades from where it was typed (`composer/useSendMotion.ts`), and Send's arrow leaves upward and returns. The field empties at once. |
| `+` open | The glyph turns 45 degrees on the spring, so the same button reads as close. |
| Switching lists | The segmented control's thumb slides on the spring. |

## Related documentation

- [Shared controls](shared-controls.md): the hover and click rule, footer cards, menu triggers.
- [Decision models](decision-models.md): what a decision chat's composer leaves out.
- [Skill catalog](../extensions/skill-catalog.md): the one composition rule for skills and Contexts.
- [Dictation surfaces](../dictation/surfaces.md): the microphone beside Send.
- [Subagents](../agent-loop/subagents.md): what a subagent inherits from the chat's per-chat switches.
