import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  PANE_HEADER_HEIGHT,
  PANE_MIN_HEIGHT,
  PANE_MIN_WIDTH,
} from '../components/Layout/yieldLadder';

/**
 * Rung 5 of the yield ladder (compact panes), at the source.
 *
 * A pane in a split that is too small for its chat shows a placeholder, and keeps the chat
 * mounted behind it in a box held at 360 by 270. That box is authored CSS the component does not
 * spell out, and jsdom applies none of it, so what is checkable here is the part that has to be
 * exactly right: the floor in CSS says the same as the constants the ladder reasons with, and each
 * declaration that keeps the hidden chat alive is present (and the ones that would break it are
 * not). `chatGroups/compactPane.browser.test.tsx` proves the pixels in Chromium, stripping each
 * declaration and expecting the defect.
 */
const STYLES = __dirname;
const MAIN = readFileSync(join(STYLES, 'main.css'), 'utf8');
const PANE_CSS_PATH = join(STYLES, '../components/chatGroups/compact-pane.css');
const PANE_CSS = existsSync(PANE_CSS_PATH) ? readFileSync(PANE_CSS_PATH, 'utf8') : '';
const BASE_CHAT = readFileSync(join(STYLES, '../components/BaseChat.tsx'), 'utf8');

const stripComments = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, '');

function token(name: string): string {
  const match = MAIN.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1].trim();
}

/** The body of the rule whose selector is exactly `selector`, comments stripped. */
function ruleBody(css: string, selector: string): string {
  const text = stripComments(css);
  const index = text.indexOf(`${selector} {`);
  if (index === -1) throw new Error(`${selector} has no rule`);
  const open = text.indexOf('{', index);
  const close = text.indexOf('}', open);
  return text.slice(open + 1, close);
}

const CONTENT_RULE = ".br-chat-pane[data-pane-fit='compact'] > .br-chat-pane__content";
const PANE_RULE = ".br-chat-pane[data-pane-fit='compact']";

describe('the compact pane floor', () => {
  it('is the ladder’s 360 by 270, in CSS', () => {
    expect(token('pane-min-width')).toBe(`${PANE_MIN_WIDTH}px`);
    expect(token('pane-min-height')).toBe(`${PANE_MIN_HEIGHT}px`);
  });

  it('builds the height floor on the band the pane really draws', () => {
    expect(token('chrome-height')).toBe(`${PANE_HEADER_HEIGHT}px`);
  });
});

describe('the hidden chat', () => {
  it('leaves the flow at the floor, hidden from paint but still laid out', () => {
    const body = ruleBody(PANE_CSS, CONTENT_RULE);
    expect(body).toMatch(/position:\s*absolute;/);
    expect(body).toMatch(/width:\s*max\(100%,\s*var\(--pane-min-width\)\);/);
    expect(body).toMatch(/height:\s*max\(100%,\s*var\(--pane-min-height\)\);/);
    expect(body).toMatch(/visibility:\s*hidden;/);
    // A descendant that sets `visibility: visible` (the fade-out keyframes do) stays hidden.
    expect(body).toMatch(/opacity:\s*0;/);
  });

  it('is never display: none, which resets the scroll and hands every observer a 0px box', () => {
    const rules = stripComments(PANE_CSS)
      .split('}')
      .filter((rule) => rule.includes('.br-chat-pane__content'));
    expect(rules.length).toBeGreaterThan(0);
    for (const rule of rules) expect(rule).not.toMatch(/display:\s*none/);
  });

  it('is clipped by a pane that is overflow: hidden, not clip', () => {
    const body = ruleBody(PANE_CSS, PANE_RULE);
    expect(body).toMatch(/overflow:\s*hidden;/);
    expect(body).not.toMatch(/clip/);
  });
});

describe('the compact pane stylesheet', () => {
  it('draws with tokens only: no colour literal', () => {
    const css = stripComments(PANE_CSS);
    expect(css.length).toBeGreaterThan(0);
    expect(css).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla|oklch|oklab)\(/i);
    expect(css).not.toMatch(/:\s*(white|black|red)\b/i);
  });

  it('draws no danger tone: a hidden chat is not an error', () => {
    expect(stripComments(PANE_CSS)).not.toMatch(/danger/);
  });
});

describe('what CompactPane mirrors from BaseChat', () => {
  it('the composer bar the height floor is built on is still pt-7 and pb-6', () => {
    expect(BASE_CHAT).toContain('biorouter-chat-composer-bar flex-shrink-0 pb-6 pt-7');
  });

  it('the dictation key is still the tab and session expression CompactPane computes', () => {
    expect(BASE_CHAT).toContain('existingChatComposerDraftKey(terminalKey, sessionId)');
    expect(BASE_CHAT).toContain('composerDraftKeyForTab(terminalKey)');
  });
});
