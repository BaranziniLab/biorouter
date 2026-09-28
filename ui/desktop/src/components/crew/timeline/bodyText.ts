import { revealHiddenCharacters, type RevealedSegment } from '../../../utils/untrustedText';
import { timelineCopy } from './copy';

/**
 * What a message body's words become once parsed (`MessageBody`'s rehype step): the text as it was
 * written, with two things drawn differently.
 *
 * 1. **Hidden characters are shown** (QA M3, SEC-9). A bidi override, an isolate, a control
 *    character, a stray tag character or a zero-width character inside a name is drawn as its
 *    escape (`\u{202e}`, as `biorouter crew` prints it) in a `crew-md-hidden-char` span, so
 *    `invoice_{U+202E}gnp.exe` no longer reads `invoice_exe.png`, and `@cre{U+200B}w_bob` no
 *    longer looks like `@crew_bob`. The span keeps the raw character (`data.crewRaw`) for whatever
 *    reads the text back rather than drawing it. The rule is `revealHiddenCharacters`, in
 *    `utils/untrustedText.ts`.
 * 2. **A mention of the viewer is marked** (QA M2): `@{their username}`, in any case, as a whole
 *    word, outside code and link text, becomes a `crew-md-mention` chip. When the body holds one,
 *    a hidden "mentions you" span with `mentionLabelId` is added at its end, which the row names
 *    itself by (`aria-labelledby`), and the row's stylesheet gives the row its accent.
 *
 * A fenced code block is left alone here: `CodeBlock` draws its text through `VisibleText` and
 * copies the raw text. Nothing here changes what is stored, sent or copied.
 */

/** The parts of a hast node this step reads and writes. */
export interface BodyNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: BodyNode[];
  /** Set on a hidden character's span: the character itself. */
  data?: { crewRaw?: string };
}

export interface BodyTextOptions {
  /** The viewer's username, whose mentions are marked; null marks none. */
  mention: string | null;
  /** The ID of the hidden "mentions you" span the row is named by, when it wants one. */
  mentionLabelId: string | null;
}

/** A character a username is written in (`names.rs`); its last character is never a dot. */
const USERNAME_CHARACTER = /[A-Za-z0-9._-]/;
const USERNAME_END_CHARACTER = /[A-Za-z0-9_-]/;

function escapeForPattern(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\-]/g, '\\$&');
}

/** The pattern of `@username`, any case, or null when there is no username to mark. */
export function mentionPattern(username: string | null): RegExp | null {
  const name = username?.trim() ?? '';
  if (!name || !/^[A-Za-z0-9._-]+$/.test(name)) return null;
  return new RegExp(`@${escapeForPattern(name)}`, 'gi');
}

/**
 * Where `@username` stands as a whole mention in `text`: not inside an address (`bob@lab.org`),
 * not the start of a longer name (`@crew_bobby`, `@crew_bob.lee`), and not against a hidden
 * character (`hiddenBefore`/`hiddenAfter` say one sits just before or after `text`), which would
 * make the name another one.
 */
function mentionRanges(
  text: string,
  pattern: RegExp,
  hiddenBefore: boolean,
  hiddenAfter: boolean
): [number, number][] {
  const ranges: [number, number][] = [];
  pattern.lastIndex = 0;
  for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
    const start = match.index;
    const end = start + match[0].length;
    const before = text[start - 1];
    const after = text[end];
    const next = text[end + 1];
    const boundedBefore =
      before === undefined ? !hiddenBefore : !USERNAME_CHARACTER.test(before) && before !== '@';
    const boundedAfter =
      after === undefined
        ? !hiddenAfter
        : !USERNAME_END_CHARACTER.test(after) &&
          !(after === '.' && next !== undefined && USERNAME_END_CHARACTER.test(next));
    if (boundedBefore && boundedAfter) ranges.push([start, end]);
  }
  return ranges;
}

function textNode(value: string): BodyNode {
  return { type: 'text', value };
}

/** A hidden character, drawn as its escape. Left to right whatever the paragraph's direction. */
export function hiddenCharacterNode(
  segment: Extract<RevealedSegment, { kind: 'hidden' }>
): BodyNode {
  return {
    type: 'element',
    tagName: 'span',
    properties: {
      className: ['crew-md-hidden-char'],
      dir: 'ltr',
      title: timelineCopy.hiddenCharacter(segment.codePoint),
      dataHiddenChar: segment.codePoint,
    },
    children: [textNode(segment.escape)],
    data: { crewRaw: segment.raw },
  };
}

function mentionNode(value: string): BodyNode {
  return {
    type: 'element',
    tagName: 'span',
    properties: { className: ['crew-md-mention'], dataMention: 'you' },
    children: [textNode(value)],
  };
}

/** One text node's replacement: its words, its hidden characters, and its mentions. */
function transformText(
  value: string,
  pattern: RegExp | null,
  marking: boolean
): { nodes: BodyNode[]; mentioned: boolean } {
  const segments = revealHiddenCharacters(value);
  const nodes: BodyNode[] = [];
  let mentioned = false;
  segments.forEach((segment, index) => {
    if (segment.kind === 'hidden') {
      nodes.push(hiddenCharacterNode(segment));
      return;
    }
    const ranges =
      pattern && marking
        ? mentionRanges(
            segment.text,
            pattern,
            segments[index - 1]?.kind === 'hidden',
            segments[index + 1]?.kind === 'hidden'
          )
        : [];
    let at = 0;
    for (const [start, end] of ranges) {
      if (start > at) nodes.push(textNode(segment.text.slice(at, start)));
      nodes.push(mentionNode(segment.text.slice(start, end)));
      mentioned = true;
      at = end;
    }
    if (at < segment.text.length) nodes.push(textNode(segment.text.slice(at)));
  });
  return { nodes, mentioned };
}

/** Elements whose text is never a mention: code, and a link's words. */
const NO_MENTION = new Set(['code', 'a']);

/**
 * Rewrite the tree's text nodes in place, and say whether a mention was marked. `pre` is skipped:
 * a code block draws and copies its own text.
 */
function transformChildren(node: BodyNode, pattern: RegExp | null, marking: boolean): boolean {
  if (!Array.isArray(node.children)) return false;
  let mentioned = false;
  const next: BodyNode[] = [];
  for (const child of node.children) {
    if (child.type === 'text' && typeof child.value === 'string') {
      const replaced = transformText(child.value, pattern, marking);
      if (replaced.mentioned) mentioned = true;
      next.push(...replaced.nodes);
      continue;
    }
    if (child.type === 'element' && child.tagName !== 'pre') {
      const inner = marking && !NO_MENTION.has(child.tagName ?? '');
      if (transformChildren(child, pattern, inner)) mentioned = true;
    }
    next.push(child);
  }
  node.children = next;
  return mentioned;
}

/**
 * The rehype plugin `MessageBody` runs: hidden characters shown, the viewer's mentions marked, and,
 * when there is one, the hidden "mentions you" label added for the row.
 */
export function rehypeCrewBodyText(options: BodyTextOptions) {
  const pattern = mentionPattern(options.mention);
  return (tree: BodyNode) => {
    const mentioned = transformChildren(tree, pattern, true);
    if (mentioned && options.mentionLabelId && Array.isArray(tree.children)) {
      tree.children.push({
        type: 'element',
        tagName: 'span',
        properties: { id: options.mentionLabelId, hidden: true },
        children: [textNode(timelineCopy.mentionsYou)],
      });
    }
  };
}

/**
 * The text of a hast node. `raw`: a hidden character's own character, for what reads the words
 * back (a link's words, compared with its address); otherwise its escape, for what names something
 * on screen (a table's region), so no raw direction control reaches an accessible name.
 */
export function bodyNodeText(node: unknown, raw = false): string {
  if (!node || typeof node !== 'object') return '';
  const current = node as BodyNode;
  if (current.type === 'text' && typeof current.value === 'string') return current.value;
  if (raw && typeof current.data?.crewRaw === 'string') return current.data.crewRaw;
  return Array.isArray(current.children)
    ? current.children.map((child) => bodyNodeText(child, raw)).join('')
    : '';
}
