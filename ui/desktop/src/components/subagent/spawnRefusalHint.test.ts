import { describe, expect, it } from 'vitest';
import {
  MAKE_CHAT_PUBLIC_FROM_HISTORY,
  PUBLIC_CHILD_REFUSAL_OPENING,
  spawnRefusalHint,
} from './spawnRefusalHint';

/** DR-19's refusal as the daemon writes it (`PrivacyRefusal::PublicChildOfPrivateParent`). */
const DR19 =
  'This chat is private, so it cannot start a subagent on a public model: the task prompt is ' +
  'private-origin text, sending it would put that text on a model outside the institution, and ' +
  'this request came from the assistant rather than from the person at the keyboard.';

describe('spawnRefusalHint', () => {
  it('opens the refusal with the words the daemon writes', () => {
    expect(DR19.startsWith(PUBLIC_CHILD_REFUSAL_OPENING)).toBe(true);
  });

  it.each([
    ['the bare spawn tool', 'subagent', DR19],
    ['the prefixed spawn tool', 'workspace__subagent', DR19],
    [
      'an annotated live error',
      'workspace__subagent',
      `-32602: [tool_error kind=invalid_args retryable=false] The arguments were rejected.\n${DR19}`,
    ],
    [
      'a chat saved before the wrapper was dropped',
      'workspace__subagent',
      `\`workspace__subagent\` failed: -32602: ${DR19}`,
    ],
  ])('adds the line under DR-19 from %s', (_case, tool, error) => {
    expect(spawnRefusalHint(tool, error)).toBe(MAKE_CHAT_PUBLIC_FROM_HISTORY);
  });

  it.each([
    // R4 and DR-31 are not cleared by making the chat public.
    [
      'R4, a public chat asking for a private subagent',
      'workspace__subagent',
      'This chat is running on a public model, so it cannot delegate to a private one.',
    ],
    [
      'DR-31, a subagent under other agreements',
      'workspace__subagent',
      'This chat runs on a model covered by UCSF, so it cannot start a subagent on a model covered by no institution.',
    ],
    // Any tool can print the words; only Biorouter's own spawn tool is believed.
    ['a third-party tool printing the words', 'someext__subagent', DR19],
    ['a tool named like the extension', 'workspace__subagent_status', DR19],
    ['no tool name', undefined, DR19],
    ['no error', 'workspace__subagent', null],
  ])('adds nothing for %s', (_case, tool, error) => {
    expect(spawnRefusalHint(tool, error)).toBeNull();
  });

  it('is one plain sentence that tells the person where and what', () => {
    expect(MAKE_CHAT_PUBLIC_FROM_HISTORY).toMatch(/^If the whole chat may be public, /);
    expect(MAKE_CHAT_PUBLIC_FROM_HISTORY).toContain('make it public from History');
    expect(MAKE_CHAT_PUBLIC_FROM_HISTORY).toContain('switch it to a public model');
    expect(MAKE_CHAT_PUBLIC_FROM_HISTORY).not.toMatch(/[–—]/);
  });
});
