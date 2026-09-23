/**
 * The composer's text model for `<biorouter-ref …>` references (issue #65).
 *
 * ## Why the message text stays the single source of truth
 *
 * The composer is a plain `<textarea>`, and ChatInput keys everything off one
 * string: draft save and restore, the `?prompt=` deep link, the message queue,
 * steering, local message storage, submit. Holding references in their own
 * React state would mean teaching every one of those seams about them, and each
 * one forgotten is a reference the user attached and the agent never sees.
 *
 * So the tags live in the message text exactly as they always have, and this
 * module splits that one string into the two things the composer draws:
 *
 * * **body** — what the textarea shows. Never contains a tag, so the user never
 *   sees the markup.
 * * **refs** — what the chip rail shows, in source order.
 *
 * A tag that arrives from anywhere else — a restored draft, a deep link, a
 * queued message being edited — is normalised to the end of the text. Reference
 * extraction is position-independent on the backend, so moving it costs nothing
 * semantically, and it is what buys a body/suffix split with no per-character
 * offset mapping between what the textarea holds and what gets sent.
 *
 * ## The separator belongs to the suffix
 *
 * {@link joinComposerText} always writes `' ' + tag`, even after a body that
 * already ends in a space, and {@link splitComposerText} always removes one
 * space with the tag. That makes the round trip exact for *every* body,
 * including one ending in whitespace.
 *
 * The alternative — only adding a space when the body needs one — puts a
 * character in the textarea that the user did not type. React then finds the
 * controlled value disagreeing with the DOM, reassigns it, and the caret lands
 * after the phantom space: the next keystroke goes in the wrong place. This is
 * why the round trip is pinned by a property test over bodies that end in a
 * space, a newline and nothing at all.
 */
import { findQuotes, quoteTag, type QuoteReference } from './quotedText';
import { findRefTags, labelledRefTag, refTag, type RefKind, type RefSpan } from './resourceRefs';

export interface ComposerText {
  /** The prose, with every reference tag removed. What the textarea shows. */
  body: string;
  /** The references, in source order. What the chip rail shows. */
  refs: (RefSpan | QuoteReference)[];
}

/** Split composer text into the prose the user edits and the references. */
export function splitComposerText(text: string): ComposerText {
  const refs = [...findRefTags(text), ...findQuotes(text)].sort((a, b) => a.start - b.start);
  if (refs.length === 0) return { body: text, refs };

  let body = '';
  let cursor = 0;

  for (const ref of refs) {
    let start = ref.start;
    // Take the separator out with the tag it belongs to, so the body comes back
    // exactly as it was written.
    if (start > cursor && text[start - 1] === ' ') start -= 1;
    body += text.slice(cursor, start);
    cursor = ref.end;
  }

  return { body: body + text.slice(cursor), refs };
}

/** The message text for a body and a set of references. */
export function joinComposerText(body: string, refs: ComposerText['refs']): string {
  return refs.reduce((text, ref) => `${text} ${tagFor(ref)}`, body);
}

const tagFor = (ref: ComposerText['refs'][number]): string =>
  ref.kind === 'quote'
    ? quoteTag(ref)
    : ref.label
      ? labelledRefTag(ref.kind, ref.value, ref.label)
      : refTag(ref.kind, ref.value);

/**
 * `text` with one more reference attached.
 *
 * A reference already present is returned unchanged: picking the same skill
 * twice is a slip rather than a request for two copies, the backend dedups
 * regardless, and two identical chips would just look broken.
 */
export function appendComposerRef(
  text: string,
  kind: RefKind,
  value: string,
  label?: string
): string {
  const { body, refs } = splitComposerText(text);
  if (refs.some((ref) => ref.kind === kind && ref.value === value)) return text;

  return joinComposerText(body, [...refs, { kind, value, label } as RefSpan]);
}

/** `text` with the reference at `index` (in source order) removed. */
export function removeComposerRefAt(text: string, index: number): string {
  const { body, refs } = splitComposerText(text);
  if (index < 0 || index >= refs.length) return text;

  return joinComposerText(
    body,
    refs.filter((_, position) => position !== index)
  );
}
