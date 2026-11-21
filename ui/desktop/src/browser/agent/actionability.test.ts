import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  DESCRIBE_TARGET_FUNCTION,
  LOCATE_FUNCTION,
  boxOfQuads,
  checkElement,
  clickPoint,
  dropPointIn,
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

describe('dropPointIn (AD-17)', () => {
  const viewport = { width: 800, height: 600 };
  // An item 40 px tall, from y 100 to 140, centre (50, 120).
  const item = [[10, 100, 90, 100, 90, 140, 10, 140]];

  it('drops past the middle of the destination in the direction of travel', () => {
    // Dragged down onto it: the lower half, where a sortable puts the item after it.
    expect(dropPointIn(item, viewport, { x: 50, y: 20 })).toEqual({ x: 50, y: 130 });
    // Dragged up onto it: the upper half.
    expect(dropPointIn(item, viewport, { x: 50, y: 400 })).toEqual({ x: 50, y: 110 });
    // Mostly sideways: past the middle across.
    expect(dropPointIn(item, viewport, { x: 600, y: 118 })).toEqual({ x: 30, y: 120 });
    // From its own centre: the centre.
    expect(dropPointIn(item, viewport, { x: 50, y: 120 })).toEqual({ x: 50, y: 120 });
  });

  it('stays inside the part of the destination in view', () => {
    // From y 580 to 640, 20 px of it in view: a quarter of the visible part past its middle.
    const low = [[10, 580, 90, 580, 90, 640, 10, 640]];
    expect(dropPointIn(low, viewport, { x: 50, y: 10 })).toEqual({ x: 50, y: 595 });
    expect(
      dropPointIn([[10, 700, 90, 700, 90, 720, 10, 720]], viewport, { x: 0, y: 0 })
    ).toBeNull();
  });
});

describe('isPlainLinkClick (BB-17: the observe grant follows plain links only)', () => {
  it('admits a link to another http(s) page', () => {
    expect(isPlainLinkClick(facts({ tag: 'a', href: 'https://site-a.test/article' }))).toBe(true);
  });

  it('refuses a link that leaves the page origin: following it is acting', () => {
    // A page that injected instructions could point the agent at a link to its own server
    // carrying page content; crossing to another origin needs the act grant.
    for (const href of [
      'https://site-b.test/exfiltrate?data=secret',
      'http://site-a.test/article',
      'https://site-a.test:8443/article',
      'https://sub.site-a.test/article',
    ]) {
      expect(isPlainLinkClick(facts({ tag: 'a', href })), href).toBe(false);
    }
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

describe('LOCATE_FUNCTION (AD-17: a drag end named by a selector or words)', () => {
  // The source string main sends to the page, run here in jsdom. jsdom lays nothing out, so an
  // element is given a size unless it sits under [data-hidden].
  const locate = new Function(`return (${LOCATE_FUNCTION})`)() as (
    kind: 'selector' | 'text',
    value: string,
    scope: string | null
  ) => Element | string;
  const describeTarget = new Function(`return (${DESCRIBE_TARGET_FUNCTION})`)() as (
    this: Element
  ) => { role: string; name: string };
  const original = Element.prototype.getBoundingClientRect;
  const hadCss = 'CSS' in globalThis;
  beforeEach(() => {
    Element.prototype.getBoundingClientRect = function (this: Element) {
      const size = this.closest('[data-hidden]') ? 0 : 10;
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: size,
        bottom: size,
        width: size,
        height: size,
        toJSON: () => ({}),
      } as DOMRect;
    };
    if (!hadCss) (globalThis as { CSS?: unknown }).CSS = { escape: (value: string) => value };
    document.body.innerHTML = `
      <div id="first"><div class="item">Item 1</div><div class="item"><span>Send\n   THE  samples</span></div></div>
      <div id="second"><div class="item">Item 1</div><div class="item">Item 10</div></div>
      <div data-hidden><div class="ghost">Only hidden</div></div>`;
  });
  afterEach(() => {
    Element.prototype.getBoundingClientRect = original;
    if (!hadCss) delete (globalThis as { CSS?: unknown }).CSS;
  });
  const report = (value: Element | string) => JSON.parse(value as string);

  it('finds the one element a selector matches', () => {
    const found = locate('selector', '#second > .item:nth-child(2)', null);
    expect((found as Element).textContent).toBe('Item 10');
    expect(describeTarget.call(found as Element)).toEqual({ role: 'div', name: 'Item 10' });
  });

  it('finds the smallest element whose whole text is the words, ignoring case and spacing', () => {
    const found = locate('text', 'send the Samples', null) as Element;
    expect(found.tagName).toBe('SPAN');
    // Whole text first: "Item 1" is not "Item 10", but it is in two lists.
    expect(report(locate('text', 'Item 1', null))).toEqual({
      problem: 'several',
      count: 2,
      hidden: 0,
      near: ['#first', '#second'],
    });
    expect((locate('text', 'Item 1', '#second') as Element).textContent).toBe('Item 1');
    // No element has exactly these words: the one whose text contains them.
    expect((locate('text', 'tem 10', null) as Element).textContent).toBe('Item 10');
  });

  it('says why when nothing visible, several, or nothing at all matches', () => {
    expect(report(locate('text', 'Only hidden', null))).toMatchObject({
      problem: 'none',
      count: 0,
      hidden: 1,
    });
    expect(report(locate('selector', '.item', null))).toMatchObject({
      problem: 'several',
      count: 4,
    });
    expect(report(locate('selector', 'Item 1', null))).toMatchObject({ problem: 'syntax' });
    expect(report(locate('text', 'Item 1', '#nowhere'))).toMatchObject({ problem: 'scope-none' });
    expect(report(locate('text', 'Item 1', '::bad'))).toMatchObject({ problem: 'scope-syntax' });
  });
});
