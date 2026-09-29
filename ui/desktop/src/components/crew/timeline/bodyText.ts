import { revealHiddenCharactersAcross, type RevealedSegment } from '../../../utils/untrustedText';
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
 *    itself by (`aria-labelledby`), and the row's stylesheet gives the row its accent. In an
 *    agent's post, the daemon's own provenance line at its end (`Source: … shared by Jack Moreno
 *    (@crew_jack).`, `crates/biorouter/src/crew/source_line.rs`) names whose file the run read,
 *    never whom the post is for: it is drawn as it is and mentions no one (CLIDOCS-F2), or everyone
 *    whose file an agent read was "mentioned" and notified every time.
 *
 * Both are judged on what is drawn side by side, not one parsed text node at a time. Emphasis,
 * strike-through, inline code and a link's words sit on the line with no gap around them, so the
 * words of a paragraph, a list item, a heading or a table cell are read as one text: a zero-width
 * space wrapped in its own `*…*` inside `@crew_b*…*ob` has `b` and `o` for neighbours, as the eye
 * has, and is shown. Judged alone, its node held nothing else and it drew as `@crew_bob`.
 *
 * Raw HTML, which this surface draws as the characters typed, is text here like any other.
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
  /**
   * The body is an agent's post (it carries a `run_id`), which ends with the daemon's provenance
   * line: that last paragraph mentions no one ({@link isProvenanceLine}). Absent: false.
   */
  agentPost?: boolean;
}

/**
 * How the daemon's provenance line begins (`with_source_line` in `source_line.rs`, and the lines
 * `crew/mod.rs` builds): the files the run read (`Source: …`, `Sources: …`), or that it read none
 * (`No shared file was read for this result.` / `… for this post.`).
 */
const PROVENANCE_LINE = /^(?:Sources?:\s|No shared file was read for this (?:result|post)\.)/;

/** Whether `text`, a paragraph's words, is the daemon's provenance line. */
export function isProvenanceLine(text: string): boolean {
  return PROVENANCE_LINE.test(text.trimStart());
}

/**
 * The daemon's provenance line of an agent's post: its last block, when that is a paragraph that
 * reads as one. The daemon always writes it last, after a blank line, so nothing the model wrote
 * can come after it; a "Source:" line the model wrote above it is the model's, and is read as
 * any other words are.
 */
function provenanceParagraph(tree: BodyNode): BodyNode | null {
  const blocks = (tree.children ?? []).filter(
    (child) => !(child.type === 'text' && typeof child.value === 'string' && !child.value.trim())
  );
  const last = blocks[blocks.length - 1];
  if (!last || last.type !== 'element' || last.tagName !== 'p') return null;
  return isProvenanceLine(bodyNodeText(last, true)) ? last : null;
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
 * character (`hidden[i]` is 1 for each code unit of one), which would make the name another one.
 */
function mentionRanges(text: string, pattern: RegExp, hidden: Uint8Array): [number, number][] {
  const ranges: [number, number][] = [];
  pattern.lastIndex = 0;
  for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
    const start = match.index;
    const end = start + match[0].length;
    const before = text[start - 1];
    const after = text[end];
    const next = text[end + 1];
    const boundedBefore =
      before === undefined ||
      (hidden[start - 1] === 0 && !USERNAME_CHARACTER.test(before) && before !== '@');
    const boundedAfter =
      after === undefined ||
      (hidden[end] === 0 &&
        !USERNAME_END_CHARACTER.test(after) &&
        !(after === '.' && next !== undefined && USERNAME_END_CHARACTER.test(next)));
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

/** Elements whose text is never a mention: code, and a link's words. */
const NO_MENTION = new Set(['code', 'a']);

/**
 * Elements laid out as blocks of their own (everything `remark-gfm` and `mdast-util-to-hast` emit
 * that is not inline): the words inside one are read apart from the words outside it. Every other
 * element is inline and drawn on the line beside its neighbours with no box: `em`, `strong`,
 * `del`, `code`, `a`, `sup`, `span`.
 */
const BLOCK = new Set([
  'blockquote',
  'div',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'hr',
  'li',
  'ol',
  'p',
  'pre',
  'section',
  'table',
  'tbody',
  'td',
  'tfoot',
  'th',
  'thead',
  'tr',
  'ul',
]);
/** Elements drawn as one thing on the line, with no words of their own: an image, a task box. */
const OBJECT = new Set(['img', 'input']);
/** What stands for such an element in the text read around it: U+FFFC, the object replacement. */
const OBJECT_CHARACTER = '\u{FFFC}';

/**
 * One piece of a block's words: a text or raw node (`node`), whose mentions are marked when
 * `marking`; or, with no node, what is drawn between two of them (a line break, an image).
 */
interface Piece {
  text: string;
  node?: BodyNode;
  marking: boolean;
}

/**
 * The tree's words, gathered into runs: each run is the words of one block, in order, down through
 * every inline element. A block element ends the run around it and starts its own. `pre` is
 * skipped: a code block draws and copies its own text.
 */
function gatherRuns(tree: BodyNode, unmarked: BodyNode | null = null): Piece[][] {
  const runs: Piece[][] = [[]];
  const endRun = () => {
    if (runs[runs.length - 1].length > 0) runs.push([]);
  };
  const visit = (node: BodyNode, marking: boolean) => {
    if (!Array.isArray(node.children)) return;
    for (const child of node.children) {
      const run = runs[runs.length - 1];
      if ((child.type === 'text' || child.type === 'raw') && typeof child.value === 'string') {
        run.push({ text: child.value, node: child, marking });
        continue;
      }
      if (child.type !== 'element') continue;
      const tag = child.tagName ?? '';
      const inner = marking && !NO_MENTION.has(tag) && child !== unmarked;
      if (tag === 'br') run.push({ text: '\n', marking: false });
      else if (OBJECT.has(tag)) run.push({ text: OBJECT_CHARACTER, marking: false });
      else if (BLOCK.has(tag)) {
        endRun();
        if (tag !== 'pre') visit(child, inner);
        endRun();
      } else visit(child, inner);
    }
  };
  visit(tree, true);
  return runs.filter((run) => run.some((piece) => piece.node));
}

/**
 * One run's replacement nodes, by the node each replaces, and whether a mention was marked. The
 * hidden characters are found across the whole run, and so are the mentions: a mention is marked
 * when it lies inside one node where mentions are marked, and its neighbours in the run, inside
 * that node or not, leave it a whole name.
 */
function rewriteRun(
  run: Piece[],
  pattern: RegExp | null,
  replacements: Map<BodyNode, BodyNode[]>
): boolean {
  const revealed = revealHiddenCharactersAcross(run.map((piece) => piece.text));
  // The run as drawn, with where each piece starts and a flag on every code unit of a hidden one.
  const starts: number[] = [];
  let flat = '';
  const hiddenAt: number[] = [];
  revealed.forEach((segments) => {
    starts.push(flat.length);
    for (const segment of segments) {
      if (segment.kind === 'hidden') {
        for (let unit = 0; unit < segment.raw.length; unit += 1) hiddenAt.push(flat.length + unit);
        flat += segment.raw;
      } else flat += segment.text;
    }
  });
  starts.push(flat.length);
  const hidden = new Uint8Array(flat.length);
  for (const unit of hiddenAt) hidden[unit] = 1;
  const mentions = pattern ? mentionRanges(flat, pattern, hidden) : [];

  let mentioned = false;
  // Each mention is taken once, in run order: a piece keeps those that lie wholly inside it.
  let nextMention = 0;
  run.forEach((piece, index) => {
    const start = starts[index];
    const end = starts[index + 1];
    const own: [number, number][] = [];
    while (nextMention < mentions.length && mentions[nextMention][0] < end) {
      const [from, to] = mentions[nextMention];
      if (piece.marking && from >= start && to <= end) own.push([from, to]);
      nextMention += 1;
    }
    if (!piece.node) return;
    const nodes: BodyNode[] = [];
    let offset = start;
    let ownIndex = 0;
    for (const segment of revealed[index]) {
      if (segment.kind === 'hidden') {
        nodes.push(hiddenCharacterNode(segment));
        offset += segment.raw.length;
        continue;
      }
      const to = offset + segment.text.length;
      let at = offset;
      // A mention holds no hidden character, so each lies inside one of these text segments.
      while (ownIndex < own.length && own[ownIndex][1] <= to) {
        const [mentionStart, mentionEnd] = own[ownIndex];
        if (mentionStart > at) nodes.push(textNode(flat.slice(at, mentionStart)));
        nodes.push(mentionNode(flat.slice(mentionStart, mentionEnd)));
        mentioned = true;
        at = mentionEnd;
        ownIndex += 1;
      }
      if (at < to) nodes.push(textNode(flat.slice(at, to)));
      offset = to;
    }
    replacements.set(piece.node, nodes);
  });
  return mentioned;
}

/** Put each replaced node's replacement in its place, anywhere in the tree. */
function replaceNodes(node: BodyNode, replacements: Map<BodyNode, BodyNode[]>): void {
  if (!Array.isArray(node.children)) return;
  const next: BodyNode[] = [];
  for (const child of node.children) {
    const replacement = replacements.get(child);
    if (replacement) {
      next.push(...replacement);
      continue;
    }
    replaceNodes(child, replacements);
    next.push(child);
  }
  node.children = next;
}

/**
 * Rewrite the tree's words in place, and say whether a mention was marked.
 *
 * Raw HTML is text too, and is rewritten here as text. With `allowDangerousHtml`, an HTML block, an
 * inline tag (attributes and all) or a comment reaches this step as a `raw` node, and react-markdown
 * turns it into a text node only afterwards, as it builds the elements. Left for that, its hidden
 * characters were drawn live: `<div>invoice_{U+202E}gnp.exe</div>` read `invoice_exe.png`. So a
 * raw node is replaced by the same text nodes and escapes as any other text, and react-markdown
 * finds no raw node left to convert.
 */
function rewriteTree(tree: BodyNode, pattern: RegExp | null, agentPost: boolean): boolean {
  const replacements = new Map<BodyNode, BodyNode[]>();
  let mentioned = false;
  const unmarked = agentPost ? provenanceParagraph(tree) : null;
  for (const run of gatherRuns(tree, unmarked))
    if (rewriteRun(run, pattern, replacements)) mentioned = true;
  replaceNodes(tree, replacements);
  return mentioned;
}

/**
 * The rehype plugin `MessageBody` runs: hidden characters shown, the viewer's mentions marked, and,
 * when there is one, the hidden "mentions you" label added for the row.
 */
export function rehypeCrewBodyText(options: BodyTextOptions) {
  const pattern = mentionPattern(options.mention);
  return (tree: BodyNode) => {
    const mentioned = rewriteTree(tree, pattern, options.agentPost === true);
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
 * The text of a hast node. `raw`: the words as they were written, for what reads them back (a
 * link's words, compared with its address): a hidden character's own character, and an image's
 * alt text as words of their own, since the image draws it (`Image: https://www.ucsf.edu`) and the
 * alt is a property no text node holds. Otherwise a hidden character's escape and no alt, for what
 * names something on screen (a table's region), so no raw direction control reaches an accessible
 * name.
 */
export function bodyNodeText(node: unknown, raw = false): string {
  if (!node || typeof node !== 'object') return '';
  const current = node as BodyNode;
  if (current.type === 'text' && typeof current.value === 'string') return current.value;
  if (raw && typeof current.data?.crewRaw === 'string') return current.data.crewRaw;
  if (raw && current.type === 'element' && current.tagName === 'img') {
    const alt = current.properties?.alt;
    return typeof alt === 'string' && alt ? ` ${alt} ` : '';
  }
  return Array.isArray(current.children)
    ? current.children.map((child) => bodyNodeText(child, raw)).join('')
    : '';
}
