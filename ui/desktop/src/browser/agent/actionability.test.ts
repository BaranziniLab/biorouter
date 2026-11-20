import { describe, expect, it } from 'vitest';
import {
  boxOfQuads,
  checkElement,
  clickPoint,
  isPlainLinkClick,
  isProtectedInput,
  isStable,
  type ElementFacts,
} from './actionability';

const facts = (extra: Partial<ElementFacts> = {}): ElementFacts => ({
  connected: true,
  tag: 'button',
  visibility: 'visible',
  display: 'inline-block',
  disabled: false,
  href: null,
  download: false,
  type: '',
  autocomplete: '',
  editable: false,
  readOnly: false,
  role: '',
  pageUrl: 'https://site-a.test/form',
  ...extra,
});
const box = { x: 10, y: 10, width: 80, height: 20 };

describe('checkElement (BB-13 step 5)', () => {
  it('passes a visible, enabled element', () => {
    expect(checkElement(facts(), box, true, '@e1')).toEqual({ ok: true });
  });

  it('fails a hidden, sizeless, detached or disabled element with its reason', () => {
    expect(checkElement(facts({ visibility: 'hidden' }), box, true, '@e1')).toEqual({
      ok: false,
      reason: '@e1 is hidden, so it cannot be used.',
    });
    expect(checkElement(facts({ display: 'none' }), box, true, '@e1').ok).toBe(false);
    expect(checkElement(facts(), { ...box, width: 0 }, true, '@e1')).toEqual({
      ok: false,
      reason: '@e1 has no size on the page (it is hidden or collapsed).',
    });
    expect(checkElement(facts({ connected: false }), box, true, '@e1').ok).toBe(false);
    expect(checkElement(facts({ disabled: true }), box, true, '@e1')).toEqual({
      ok: false,
      reason: '@e1 is disabled.',
    });
    // A hover does not need the element enabled.
    expect(checkElement(facts({ disabled: true }), box, false, '@e1').ok).toBe(true);
  });
});

describe('geometry', () => {
  it('reads a box and its stability', () => {
    expect(boxOfQuads([[10, 10, 90, 10, 90, 30, 10, 30]])).toEqual(box);
    expect(isStable(box, { ...box })).toBe(true);
    expect(isStable(box, { ...box, y: 40 })).toBe(false);
    expect(isStable(null, box)).toBe(false);
  });

  it('aims at the centre of the visible part of the first usable quad', () => {
    expect(clickPoint([[10, 10, 90, 10, 90, 30, 10, 30]], { width: 800, height: 600 })).toEqual({
      x: 50,
      y: 20,
    });
    // Off the bottom of the viewport: nothing to click.
    expect(
      clickPoint([[10, 700, 90, 700, 90, 720, 10, 720]], { width: 800, height: 600 })
    ).toBeNull();
    // Half off the right edge: the centre of the part in view.
    expect(clickPoint([[780, 0, 820, 0, 820, 20, 780, 20]], { width: 800, height: 600 })).toEqual({
      x: 790,
      y: 10,
    });
  });
});

describe('isPlainLinkClick (BB-17: the observe grant follows plain links only)', () => {
  it('admits a link to another http(s) page', () => {
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'https://site-a.test/article' }))).toBe(true);
  });

  it('refuses buttons, downloads, fragment links and other schemes', () => {
    expect(isPlainLinkClick(facts())).toBe(false);
    expect(
      isPlainLinkClick(facts({ tag: 'a', href: 'https://site-a.test/file.pdf', download: true }))
    ).toBe(false);
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'https://site-a.test/form#top' }))).toBe(false);
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'javascript:void(0)' }))).toBe(false);
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'mailto:a@example.org' }))).toBe(false);
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'https://u:p@site-a.test/' }))).toBe(false);
  });
});

describe('isProtectedInput (BB-13 step 2)', () => {
  it('protects passwords, one time codes and card fields', () => {
    expect(isProtectedInput({ tag: 'input', type: 'password', autocomplete: '' })).toBe(true);
    expect(isProtectedInput({ tag: 'input', type: 'text', autocomplete: 'one-time-code' })).toBe(
      true
    );
    expect(
      isProtectedInput({ tag: 'input', type: 'text', autocomplete: 'section-pay cc-number' })
    ).toBe(true);
    expect(isProtectedInput({ tag: 'input', type: 'text', autocomplete: 'new-password' })).toBe(
      true
    );
    expect(isProtectedInput({ tag: 'input', type: 'text', autocomplete: 'email' })).toBe(false);
    expect(isProtectedInput({ tag: 'textarea', type: '', autocomplete: '' })).toBe(false);
  });
});
