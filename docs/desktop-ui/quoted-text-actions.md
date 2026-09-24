# Quote selected text into a conversation

> **What this is.** How a reader turns a selection in an agent response — or in a
> document, code or HTML preview — into a removable, source-labelled quotation on
> that conversation's composer, and what the feature deliberately does not do.
> **Status:** Current.
> **Audience:** Anyone working on the composer, the preview panel or the quote
> wire format.

Select text in an agent response, then right-click and choose **Ask about it** or **Quote it**. Both actions attach a removable, source-labelled quotation to that conversation’s composer and focus the question field. Neither sends a message. Existing draft text, attachments, resource references and queued messages stay intact.

Word, Markdown, plain text, source-code and rendered HTML previews support the same flow. The quote button beside the preview capture controls attaches the current selection. HTML selections cross the existing sandbox through a bounded data-only message; attaching still requires an action in the host UI. Canvas-only PDFs and live external browser views do not currently expose text selection to this control.

Quotes preserve the original selected text and source locator/revision when available. The composer accepts at most 16,000 UTF-16 code units per quotation and explains oversized selections instead of truncating them. Quotes remain editable as removable chips after tab-switch draft restoration and queue editing. Existing-chat drafts are owned by both tab and session, retained in renderer memory while that tab remains open, and discarded on close or reload; no unsent text is newly persisted to disk. In split layouts, the originating chat group determines the composer; ambiguous destinations fail visibly.

The wire format uses a JSON-escaped `biorouter-quote` envelope. Validated quotation blocks are excluded from deterministic resource-reference extraction, so examples such as `/ext:developer`, `/skill(example)` and `kb_id:fixture` remain source data. Explicit resource references outside a quote still work normally.

Recents now offers **Delete conversation** with a permanent-deletion confirmation. It shares History’s authorized API request, cache removal and open-tab notification. **Copy conversation ID** uses a native write-only clipboard bridge restricted to registered app main frames; embedded frames retain no clipboard access.

## Related documentation

- [Artifact display surfaces](artifact-display-surfaces.md) — the preview panel a
  quote can be taken from, and why an artifact is shown in exactly one place.
- [Launching the dev GUI from a shell without a TTY](launching-the-dev-gui.md) —
  how to get the app in front of you to exercise any of this.
