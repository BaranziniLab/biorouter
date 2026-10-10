import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Every card in the chat transcript is set at the transcript's own size.
 *
 * `biorouter-message-content` marks a block that renders INSIDE the transcript,
 * shoulder to shoulder with the assistant's prose. That prose is 14px. The
 * document default is 16px, so a card that names no size class does not render
 * "at the default" — it renders two pixels larger than everything around it,
 * which is what the user saw and described as a banner sitting among the text.
 *
 * The bug is invisible to every component test in this repo, and not by
 * accident: jsdom has no layout engine and never runs Tailwind, so a card
 * rendered in a test reports the same (absent) computed size whether or not the
 * class is there. Nothing short of a real browser can measure it. So the
 * assertion is made where it CAN be made — over the sources — and it is made as
 * a rule about the marker rather than as a list of the six files that were
 * wrong on the day it was written.
 *
 * The rule: a `biorouter-message-content` element states its own type size.
 * Any of the semantic roles satisfies it, as does a raw Tailwind step; the
 * point is that the size is a decision, not an inheritance.
 */
const COMPONENTS = join(__dirname, '../components');
const MARKER = 'biorouter-message-content';
const SIZE = /\btext-(body|label|supporting|code|sm|xs|base|lg)\b/;

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return entry.isFile() && entry.name.endsWith('.tsx') && !entry.name.endsWith('.test.tsx')
      ? [path]
      : [];
  });
}

/** Every line that puts the marker on an element, as `path:line — text`. */
function markerLines(): string[] {
  return sourceFiles(COMPONENTS).flatMap((path) =>
    readFileSync(path, 'utf8')
      .split('\n')
      .map((text, index) => ({ text, line: index + 1 }))
      .filter(({ text }) => text.includes(MARKER))
      .map(({ text, line }) => `${path.slice(COMPONENTS.length + 1)}:${line} — ${text.trim()}`)
  );
}

describe('chat-surface cards state their own type size', () => {
  it('finds the marker in the tree at all', () => {
    // Guards the guard: a renamed marker would empty the sweep below, and an
    // empty sweep passes. This is the assertion that fails instead.
    expect(markerLines().length).toBeGreaterThan(5);
  });

  it('never ships a card that inherits the 16px document default', () => {
    const unsized = markerLines().filter((line) => !SIZE.test(line));
    expect(unsized).toEqual([]);
  });
});

/**
 * The 16px radius (`rounded-2xl`, `--radius-surface`) belongs to the preview
 * sheet alone (implementation spec 2.2: "`rounded-2xl` is banned outside
 * `artifacts/`"). Every other surface takes the 12px container radius, so a
 * card in the transcript and a card in a view read as one family. The approval,
 * elicitation and secret cards and the find bar each carried their own 16px
 * corners; this keeps a new one from arriving.
 */
describe('no 16px card radius outside the preview sheet', () => {
  /**
   * Files still on the old radius, each owned by a workstream that is moving
   * it. An entry that no longer needs to be here fails too, so the list only
   * shrinks.
   */
  const PENDING: Record<string, string> = {};

  const offenders = () =>
    sourceFiles(COMPONENTS)
      .map((path) => path.slice(COMPONENTS.length + 1))
      .filter((rel) => !rel.startsWith('artifacts/'))
      .filter((rel) => /\brounded-2xl\b/.test(readFileSync(join(COMPONENTS, rel), 'utf8')));

  it('appears in no component outside artifacts/', () => {
    expect(offenders().filter((rel) => !(rel in PENDING))).toEqual([]);
  });

  it('keeps the pending list honest', () => {
    const found = new Set(offenders());
    expect(Object.keys(PENDING).filter((rel) => !found.has(rel))).toEqual([]);
  });
});

/**
 * The transcript's own files take the type roles only (spec 2.1): five sizes,
 * each a named role, never an arbitrary `text-[Npx]` or a raw Tailwind step,
 * which is how the transcript drifted to eight sizes before the redesign.
 */
describe('the transcript speaks only the type roles', () => {
  const FILES = [
    'BioRouterMessage.tsx',
    'UserMessage.tsx',
    'MessageMeta.tsx',
    'MessageCopyLink.tsx',
    'MessageDivergeLink.tsx',
    'MarkdownContent.tsx',
    'ProgressiveMessageList.tsx',
    'ProvenanceChip.tsx',
    'SessionNamePill.tsx',
    'InlineImage.tsx',
    'WorkflowHeader.tsx',
    'TurnActivityIndicator.tsx',
    'LoadingBioRouter.tsx',
    'conversation/ChatTurnError.tsx',
    'conversation/ChatTurnStopped.tsx',
    'subagent/SubagentTabHeader.tsx',
    'subagent/SubagentComposerSlot.tsx',
    'workflows/WorkflowActivities.tsx',
    'context_management/SystemNotificationInline.tsx',
  ];
  const OFF_ROLE = /\btext-(?:\[\d+px\]|(?:xs|sm|base|lg|xl|2xl)\b)/;

  it.each(FILES)('%s', (rel) => {
    const offenders = readFileSync(join(COMPONENTS, rel), 'utf8')
      .split('\n')
      .map((text, index) => ({ text, line: index + 1 }))
      // Comments may name the old classes to say why they went.
      .filter(({ text }) => !/^\s*(\/\/|\*|\{\/\*)/.test(text))
      .filter(({ text }) => OFF_ROLE.test(text))
      .map(({ text, line }) => `${rel}:${line} — ${text.trim()}`);
    expect(offenders).toEqual([]);
  });
});
