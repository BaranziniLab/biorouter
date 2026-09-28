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

/** The tree after the body step, and how long parsing and the step took together. */
function bodyTree(body: string, mention: string | null = null) {
  let tree: BodyNode | null = null;
  const capture = () => (root: BodyNode) => {
    tree = structuredClone(root);
    // Nothing is drawn from here: an empty tree leaves react-markdown nothing to build.
    root.children = [];
  };
  const started = performance.now();
  Markdown({
    children: body,
    remarkPlugins: [remarkGfm, remarkBreaks],
    rehypePlugins: [
      [rehypeCrewBodyText as never, { mention, mentionLabelId: 'mention-label' }],
      capture as never,
    ],
  });
  const elapsed = performance.now() - started;
  if (!tree) throw new Error('the body step never ran');
  return { tree: tree as BodyNode, elapsed };
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
});
