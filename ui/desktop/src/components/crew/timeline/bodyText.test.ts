import Markdown from 'react-markdown';
import remarkBreaks from 'remark-breaks';
import remarkGfm from 'remark-gfm';
import { describe, expect, it } from 'vitest';
import { bodyNodeText, rehypeCrewBodyText, type BodyNode } from './bodyText';

/**
 * The body step on the tree `MessageBody` hands it: markdown parsed with the same plugins, raw HTML
 * reaching it as `raw` nodes (react-markdown turns those into text only later). Escapes are braced
 * so this file never holds the characters it tests.
 */

/**
 * The tree after the body step, how long parsing and the step took together (`elapsed`), and how
 * long the step took on its own (`stepElapsed`).
 */
function bodyTree(body: string, mention: string | null = null) {
  let tree: BodyNode | null = null;
  let stepStarted = 0;
  let stepElapsed = 0;
  const startStep = () => () => {
    stepStarted = performance.now();
  };
  const capture = () => (root: BodyNode) => {
    stepElapsed = performance.now() - stepStarted;
    tree = structuredClone(root);
    // Nothing is drawn from here: an empty tree leaves react-markdown nothing to build.
    root.children = [];
  };
  const started = performance.now();
  Markdown({
    children: body,
    remarkPlugins: [remarkGfm, remarkBreaks],
    rehypePlugins: [
      startStep as never,
      [rehypeCrewBodyText as never, { mention, mentionLabelId: 'mention-label' }],
      capture as never,
    ],
  });
  const elapsed = performance.now() - started;
  if (!tree) throw new Error('the body step never ran');
  return { tree: tree as BodyNode, elapsed, stepElapsed };
}

/** Every node of `type` in the tree. */
function nodesOf(node: BodyNode, type: string): BodyNode[] {
  const own = node.type === type ? [node] : [];
  return own.concat(...(node.children ?? []).map((child) => nodesOf(child, type)));
}

const hiddenIn = (tree: BodyNode) =>
  nodesOf(tree, 'element')
    .filter((node) =>
      (node.properties?.className as string[] | undefined)?.includes('crew-md-hidden-char')
    )
    .map((node) => node.data?.crewRaw);

describe('raw HTML in the body step', () => {
  it.each([
    ['an HTML block', '<div>\nOpen invoice_\u{202E}gnp.exe now\n</div>'],
    ['an inline tag’s attribute', 'Open invoice_<x a="\u{202E}gnp.exe"> now'],
    ['an HTML comment', 'Look <!-- \u{202E} --> here'],
  ])('leaves no raw node, and shows the hidden characters of %s', (_label, body) => {
    const { tree } = bodyTree(body);
    expect(nodesOf(tree, 'raw')).toEqual([]);
    expect(hiddenIn(tree)).toEqual(['\u{202E}']);
    // What reads the words back gets the characters that were sent, the HTML's included.
    expect(bodyNodeText(tree, true)).toContain(
      body.includes('<!--') ? '<!-- \u{202E} -->' : '\u{202E}gnp.exe'
    );
  });

  it('marks a mention written inside raw HTML, which is drawn as the characters typed', () => {
    const { tree } = bodyTree('<div>\nping @crew_bob\n</div>', 'crew_bob');
    const chips = nodesOf(tree, 'element').filter((node) =>
      (node.properties?.className as string[] | undefined)?.includes('crew-md-mention')
    );
    expect(chips.map((chip) => bodyNodeText(chip))).toEqual(['@crew_bob']);
  });
});

const mentionsIn = (tree: BodyNode) =>
  nodesOf(tree, 'element')
    .filter((node) =>
      (node.properties?.className as string[] | undefined)?.includes('crew-md-mention')
    )
    .map((node) => bodyNodeText(node));

/**
 * Emphasis, strike-through, inline code and a link's words are drawn on the line with nothing
 * around them, so a hidden character wrapped in one of its own sits between the letters outside it.
 * Judged one text node at a time it had no neighbour and no token, and `@crew_b*{U+200B}*ob` read
 * exactly `@crew_bob` with nothing shown.
 */
describe('a hidden character in an inline element of its own', () => {
  it.each([
    ['emphasis inside a handle', 'hi @crew_b*\u{200B}*ob', '\u{200B}'],
    ['strong emphasis inside a handle', 'hi @crew_b**\u{2060}**ob', '\u{2060}'],
    ['strike-through inside a handle', 'hi @crew_b~~\u{200B}~~ob', '\u{200B}'],
    ['a link inside a handle', 'hi @crew_b[\u{2060}](https://a.bc)ob', '\u{2060}'],
    ['inline code inside a handle', 'hi @crew_b`\u{200B}`ob', '\u{200B}'],
    ['emphasis inside an email address', 'mail bob@lab*\u{200B}*.org', '\u{200B}'],
    ['a link inside an email address', 'mail bob@lab[\u{200B}](https://a.bc).org', '\u{200B}'],
    ['emphasis inside a domain', 'visit ucsf*\u{2060}*.edu', '\u{2060}'],
    ['strike-through inside a domain', 'visit ucsf~~\u{2060}~~.edu', '\u{2060}'],
    ['a link inside a domain', 'visit ucsf[\u{200B}](https://a.bc).edu', '\u{200B}'],
    ['emphasis nested in a link', 'hi [@crew_b*\u{200B}*ob](https://a.bc)', '\u{200B}'],
    ['emphasis in a list item', '- ping @crew_b*\u{200B}*ob', '\u{200B}'],
    ['emphasis in a table cell', '| who |\n| --- |\n| @crew_b*\u{200B}*ob |', '\u{200B}'],
  ])('shows it: %s', (_label, body, hidden) => {
    const { tree } = bodyTree(body, 'crew_bob');
    expect(hiddenIn(tree)).toEqual([hidden]);
    // Neither half, nor the two together, is a mention.
    expect(mentionsIn(tree)).toEqual([]);
  });

  it.each([
    ['an emoji sequence split by emphasis', '*\u{1F469}*\u{200D}\u{1F52C} done'],
    ['a Hebrew word in strong emphasis, then its mark', 'נא לקרוא **שלום**\u{200F}.'],
    ['Thai words with emphasis beside their break', 'ภาษา*ไทย*\u{200B}ดีมาก'],
    ['a Persian non-joiner beside emphasis', 'می*\u{200C}*خواهم'],
  ])('leaves it alone: %s', (_label, body) => {
    expect(hiddenIn(bodyTree(body).tree)).toEqual([]);
  });

  it('reads each block apart: a direction mark in an English paragraph is shown beside a Hebrew one', () => {
    const { tree } = bodyTree('שלום לכולם\n\nopen file\u{200F}.txt');
    expect(hiddenIn(tree)).toEqual(['\u{200F}']);
  });

  it('marks a mention inside emphasis, and not one a neighbouring element makes longer', () => {
    const { tree } = bodyTree(
      'cc *@crew_bob*, and @crew_bob*by*, and @crew_bob*\u{200B}*',
      'crew_bob'
    );
    expect(mentionsIn(tree)).toEqual(['@crew_bob']);
    expect(hiddenIn(tree)).toEqual(['\u{200B}']);
  });

  it('keeps every word of the body, in order, with each hidden character where it was', () => {
    const body = 'hi @crew_b*\u{200B}*ob, see [ucsf*\u{2060}*.edu](https://a.bc) and `x\u{200B}y`';
    const { tree } = bodyTree(body, 'crew_bob');
    expect(bodyNodeText(tree, true)).toBe(
      'hi @crew_b\u{200B}ob, see ucsf\u{2060}.edu and x\u{200B}y'
    );
  });
});

/**
 * The step runs on the renderer's main thread as each row mounts, on up to 64 KB somebody else
 * chose (the broker's limit). Before the hidden-character rule was one pass, `@crew_bob` and
 * 21,700 zero-width spaces took 20 s here.
 */
describe('the body step on a 64 KB body built to be slow', () => {
  const LIMIT_BYTES = 64 * 1024;
  const bytes = (value: string) => new TextEncoder().encode(value).length;
  const fill = (unit: string, lead = '') =>
    lead + unit.repeat(Math.floor((LIMIT_BYTES - bytes(lead)) / bytes(unit)));

  it.each([
    ['a mention, then zero-width spaces', fill('\u{200B}', '@crew_bob')],
    ['letters and zero-width spaces', fill('a\u{200B}')],
    ['a flag, then tag characters', fill('\u{E0067}', '\u{1F3F4}')],
    ['an HTML block of zero-width non-joiners', `<div>\n${fill('b\u{200C}', 'x')}\n</div>`],
  ])('%s', (_label, body) => {
    const { tree, elapsed } = bodyTree(body, 'crew_bob');
    expect(hiddenIn(tree).length).toBeGreaterThan(1000);
    expect(elapsed).toBeLessThan(3000);
  });

  /**
   * A paragraph's pieces are read together: thousands of them must cost the step no more than one
   * text of the same length. Timed on the step alone, since parsing this much inline markup is the
   * markdown parser's cost (about 3 s for emphasis, on HEAD as here), not this step's.
   */
  it.each([
    ['inline code around zero-width spaces', fill('a`\u{200B}`')],
    ['links around zero-width spaces', fill('a[\u{200B}](https://a.bc)')],
  ])('%s, split into thousands of pieces', (_label, body) => {
    const { tree, stepElapsed } = bodyTree(body, 'crew_bob');
    expect(hiddenIn(tree).length).toBeGreaterThan(1000);
    expect(stepElapsed).toBeLessThan(750);
  });

  it.each([
    ['mentions between runs of zero-width spaces', fill('@crew_bob \u{200B}\u{200B} ')],
    ['mentions, each in a piece of its own', fill('@crew_bob `x` ')],
  ])('%s', (_label, body) => {
    const { tree, stepElapsed } = bodyTree(body, 'crew_bob');
    expect(mentionsIn(tree).length).toBeGreaterThan(1000);
    expect(stepElapsed).toBeLessThan(750);
  });
});
