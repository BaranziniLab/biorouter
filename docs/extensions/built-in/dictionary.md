# Dictionary

> **What this is.** The built-in capability that proposes words for your personal dictionary.
> **Status:** Current.
> **Audience:** People using Biorouter dictation.

The Dictionary capability starts off. At your first speech model install, **Suggest dictionary words from my chats** lets you turn it on for new chats. You can change that choice later on the Dictionary page. Existing chats keep their current tools.

## What the agent can do

The agent has three tools when this capability is on:

| Tool | What it does |
|---|---|
| `dictionary__add_terms` | Proposes up to ten words from the conversation, with an optional kind and reason. |
| `dictionary__list_terms` | Lists words this chat may see, up to 50 at a time. Private words from another institution are left out. |
| `dictionary__remove_added_terms` | Removes only agent words that still await your review. |

The agent cannot add aliases, activate a suggestion, change your words, import or export, or install a model. File tools cannot read or change the dictionary store. This capability is unavailable to app sessions, scheduled runs, subagents, hidden sessions, terminal sessions, Crew scoped chats and ACP WebSocket sessions. ACP stdio may propose words, but its writes always wait for review.

## Your choices

**Agent's terms** on the Dictionary page controls what an agent write does:

- **Add when I ask** is the default. A word in your latest message, or in an agent offer that your latest message directly answers, becomes active and awaits your review when the agent marks it requested by you. Other words are suggestions. The store checks whether the word was named, but does not judge whether your reply agreed with the offer.
- **Suggest for review** keeps every agent word suggested until you accept it.
- **Add at once** makes agent words active and unreviewed until the per-tier cap is reached; further words become suggestions.

An active unreviewed word shapes dictation. Its tool row has **Keep** and **Undo**, and the Dictionary page places it under **Added by the agent**. A suggested word has **Add to dictionary** on the tool row and appears under **Suggested**. **Review in Dictionary** opens the page. An agent word uses exact matching and carries no spoken forms or misrecognitions.

The capability is separate from **Also find terms in my messages automatically**, which is controlled by the chat extractor.

## Example

You can say, “Please add EPAS1 to my dictionary.” If the agent calls `dictionary__add_terms` for EPAS1 with `requested_by_user`, the default setting makes it active and unreviewed. You can press **Keep** or **Undo** beside the tool call.

## Related documentation

- [Built-in capabilities](README.md)
- [The personal dictionary](../../dictation/dictionary.md)
- [Dictation surfaces](../../dictation/surfaces.md)
