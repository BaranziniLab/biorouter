import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, render } from '@testing-library/react';
import { fireEvent } from '@testing-library/dom';
import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * #37 — the FLIP pass: on reorder/close, tabs get an INVERTED inline translate
 * and are released to identity with the spring easing, instead of teleporting.
 *
 * The offset rides the INDIVIDUAL `translate` property, never the `transform`
 * list (Codex B6 re-review finding 4): the br-tab-select settle animates the
 * individual `scale`, and the CTM composes translate → rotate → scale →
 * `transform` — a translateX() in `transform` would sit inside that scale and
 * be shrunk with it, starting the active successor off-slot.
 *
 * jsdom computes no layout (offsetLeft is 0 everywhere), so these tests stub
 * offsetLeft to a synthetic 100px grid — they verify the WIRING (measure →
 * invert → release → clean up), not the pixels. The visual spring itself is
 * browser-verified, per the repo's Prism/Tailwind lesson.
 */

const TAB_SLOT_PX = 100;

function tab(id: string): ChatTab {
  return { tabId: id, sessionId: `s-${id}`, title: `Chat ${id}`, userSetName: false };
}

function renderStrip(tabs: ChatTab[], over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs,
    activeTabId: tabs[0]?.tabId ?? null,
    runningSessionIds: [],
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onReorder: vi.fn(),
    reserveTitlebar: false,
    isCompactSidebarOverlayOpen: false,
    ...over,
  };
  const utils = render(<ChatTabStrip {...props} />);
  return { ...utils, props };
}

function tabNode(container: HTMLElement, tabId: string) {
  return container.querySelector(`[data-tab-id="${tabId}"]`) as HTMLElement;
}

const originalOffsetLeft = Object.getOwnPropertyDescriptor(
  HTMLElement.prototype,
  'offsetLeft'
) as PropertyDescriptor;
const originalMatchMedia = window.matchMedia;

// A cancellable rAF stub: the FLIP cleanup calls cancelAnimationFrame, and a
// cancelled frame must never run — exactly the browser contract.
let frames = new Map<number, Parameters<typeof window.requestAnimationFrame>[0]>();
let nextFrameId = 1;

function flushFrames() {
  act(() => {
    const pending = [...frames.values()];
    frames.clear();
    for (const cb of pending) cb(performance.now());
  });
}

beforeEach(() => {
  frames = new Map();
  nextFrameId = 1;
  vi.spyOn(window, 'requestAnimationFrame').mockImplementation((cb) => {
    const id = nextFrameId;
    nextFrameId += 1;
    frames.set(id, cb);
    return id;
  });
  vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id: number) => {
    frames.delete(id);
  });
  // A synthetic layout: each tab occupies a 100px slot in its DOM order.
  Object.defineProperty(HTMLElement.prototype, 'offsetLeft', {
    configurable: true,
    get(this: HTMLElement) {
      if (!this.dataset?.tabId || !this.parentElement) return 0;
      const siblings = Array.from(this.parentElement.querySelectorAll(':scope > [data-tab-id]'));
      return siblings.indexOf(this) * TAB_SLOT_PX;
    },
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  Object.defineProperty(HTMLElement.prototype, 'offsetLeft', originalOffsetLeft);
  window.matchMedia = originalMatchMedia;
});

describe('ChatTabStrip — FLIP slide on reorder (#37)', () => {
  it('applies the INVERTED translate when the order changes, then releases with the spring', () => {
    const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);

    // Mount pass: a snapshot, no motion.
    expect(tabNode(container, 'a').style.translate).toBe('');
    expect(tabNode(container, 'b').style.translate).toBe('');

    rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);

    // First: each moved tab is inverted back to where it WAS (a: 0 → 100px slot
    // means translate: -100px; b: 100 → 0 means translate: 100px), with
    // transitions off so the jump is invisible.
    const a = tabNode(container, 'a');
    const b = tabNode(container, 'b');
    expect(a.style.translate).toBe(`${-TAB_SLOT_PX}px`);
    expect(b.style.translate).toBe(`${TAB_SLOT_PX}px`);
    expect(a.style.transition).toBe('none');
    expect(b.style.transition).toBe('none');
    // Finding 4: the offset must NOT ride the `transform` list — in the CTM
    // the standalone br-tab-select `scale` composes before `transform`, so a
    // translateX() there would be scaled during the settle and the sliding tab
    // would start away from its previous slot.
    expect(a.style.transform).toBe('');
    expect(b.style.transform).toBe('');

    // Next frame: released to identity under the spring easing.
    flushFrames();
    expect(a.style.translate).toBe('');
    expect(b.style.translate).toBe('');
    expect(a.style.transition).toBe('translate var(--dur-fast-max) var(--ease-spring)');
    expect(b.style.transition).toBe('translate var(--dur-fast-max) var(--ease-spring)');

    // Once the transition settles, the inline transition is handed back to the
    // stylesheet — no residue.
    fireEvent.transitionEnd(a);
    fireEvent.transitionEnd(b);
    expect(a.style.transition).toBe('');
    expect(a.style.translate).toBe('');
  });

  it('the shift after a CLOSE slides the survivors leftward', () => {
    const { container, rerender, props } = renderStrip([tab('a'), tab('b'), tab('c')]);

    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('c')]} />);

    // a kept slot 0: untouched. c moved 200 → 100: inverted +100px.
    expect(tabNode(container, 'a').style.translate).toBe('');
    expect(tabNode(container, 'c').style.translate).toBe(`${TAB_SLOT_PX}px`);
  });

  it('an active SUCCESSOR slides on translate with transform untouched (finding 4)', () => {
    // The exact overlap the Codex re-review flagged: closing the active tab
    // makes the successor gain data-active (starting the br-tab-select scale
    // settle) in the same commit the FLIP pass writes its slide offset on the
    // same element. The offset must ride the individual `translate` property —
    // `transform` stays untouched through invert AND release, so the settle
    // scale (which composes before the transform list) can never shrink it.
    const { container, rerender, props } = renderStrip([tab('a'), tab('b'), tab('c')], {
      activeTabId: 'b',
    });

    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('c')]} activeTabId="c" />);

    const c = tabNode(container, 'c');
    expect(c.dataset.active).toBe('true'); // settling…
    expect(c.style.translate).toBe(`${TAB_SLOT_PX}px`); // …while inverted…
    expect(c.style.transform).toBe(''); // …with the transform list untouched.

    flushFrames();
    expect(c.style.translate).toBe('');
    expect(c.style.transform).toBe('');
    expect(c.style.transition).toBe('translate var(--dur-fast-max) var(--ease-spring)');
  });

  it('a newly opened tab grows in where it lands; it does not slide in from nowhere', () => {
    const { container, rerender, props } = renderStrip([tab('a')]);

    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} />);

    expect(tabNode(container, 'a').style.translate).toBe('');
    expect(tabNode(container, 'b').style.translate).toBe('');
    // Its entrance is the CSS keyframe the attribute selects (spec 3.16).
    expect(tabNode(container, 'b').dataset.tabEntering).toBe('true');
    expect(tabNode(container, 'a').dataset.tabEntering).toBeUndefined();
  });

  it('respects prefers-reduced-motion: no translate is ever applied', () => {
    window.matchMedia = ((query: string) =>
      ({
        matches: query === '(prefers-reduced-motion: reduce)',
        media: query,
        onchange: null,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList) as typeof window.matchMedia;

    const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
    rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);

    expect(tabNode(container, 'a').style.translate).toBe('');
    expect(tabNode(container, 'b').style.translate).toBe('');
    expect(frames.size).toBe(0);
  });

  it('an unchanged re-render leaves every tab untouched', () => {
    const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} />);

    expect(tabNode(container, 'a').style.translate).toBe('');
    expect(tabNode(container, 'b').style.translate).toBe('');
  });

  it('the active tab still carries data-active (the accent strip is CSS-only)', () => {
    const { container } = renderStrip([tab('a'), tab('b')], { activeTabId: 'b' });
    expect(tabNode(container, 'b').dataset.active).toBe('true');
    expect(tabNode(container, 'a').dataset.active).toBeUndefined();
  });

  // Codex review B6 finding 6 — interrupted animations must not leak: pending
  // release frames are cancelled and inline overrides restored by the effect
  // cleanup, and transitioncancel is handled like transitionend.
  describe('cleanup of interrupted animations', () => {
    it('unmount mid-animation cancels pending frames and restores inline styles', () => {
      const { container, rerender, props, unmount } = renderStrip([tab('a'), tab('b')]);
      rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);

      const a = tabNode(container, 'a');
      expect(a.style.translate).toBe(`${-TAB_SLOT_PX}px`); // inverted
      expect(frames.size).toBeGreaterThan(0); // release frames scheduled

      unmount();

      // Every scheduled release frame was cancelled — nothing left to fire on
      // a torn-down strip…
      expect(frames.size).toBe(0);
      // …and the detached nodes carry no inline residue.
      expect(a.style.translate).toBe('');
      expect(a.style.transition).toBe('');
    });

    it('transitioncancel (an interrupted slide) restores the stylesheet transition', () => {
      const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
      rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);
      flushFrames();

      const a = tabNode(container, 'a');
      expect(a.style.transition).toBe('translate var(--dur-fast-max) var(--ease-spring)');

      fireEvent(a, new Event('transitioncancel', { bubbles: true }));
      expect(a.style.transition).toBe('');
      expect(a.style.translate).toBe('');
    });

    it('a bubbling transitionend from a CHILD does not cut the slide short', () => {
      const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
      rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);
      flushFrames();

      const a = tabNode(container, 'a');
      const child = a.querySelector('button') as HTMLElement; // e.g. the close control's opacity fade
      fireEvent.transitionEnd(child);
      expect(a.style.transition).toBe('translate var(--dur-fast-max) var(--ease-spring)'); // still sliding

      fireEvent.transitionEnd(a);
      expect(a.style.transition).toBe(''); // its own end still cleans up
    });

    it('a second reorder mid-animation restores the first pass before inverting again', () => {
      const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
      rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a')]} />);
      expect(frames.size).toBe(2); // two release frames from pass 1

      // Interrupt before the release frame ever ran.
      rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} />);

      // Pass 1's frames were cancelled and replaced by pass 2's…
      expect(frames.size).toBe(2);
      // …and the tabs carry pass 2's fresh inversion, transitions off.
      const a = tabNode(container, 'a');
      const b = tabNode(container, 'b');
      expect(a.style.translate).toBe(`${TAB_SLOT_PX}px`);
      expect(b.style.translate).toBe(`${-TAB_SLOT_PX}px`);
      expect(a.style.transition).toBe('none');

      // The interrupted pass leaves nothing behind: releasing and ending pass
      // 2 clears every inline override.
      flushFrames();
      fireEvent.transitionEnd(a);
      fireEvent.transitionEnd(b);
      expect(a.style.transition).toBe('');
      expect(a.style.translate).toBe('');
      expect(b.style.transition).toBe('');
      expect(b.style.translate).toBe('');
    });
  });
});

/**
 * Spec 3.16, "Opening tabs": a tab that OPENS grows in from its left edge
 * (`data-tab-entering` selects the `br-tab-enter` keyframe in main.css), and a
 * tab the strip MOUNTS with does not: a reload restoring its tabs, a window
 * opening with them and a tear-off all appear at rest. jsdom runs no CSS, so
 * these pin the attribute's life; the keyframe itself is pinned at the source
 * below and seen in the vision pass.
 */
describe('ChatTabStrip: the new-tab entrance (spec 3.16)', () => {
  it('a strip mounts with its tabs at rest: no entrance on a restore or a new window', () => {
    const { container } = renderStrip([tab('a'), tab('b'), tab('c')]);
    for (const id of ['a', 'b', 'c']) {
      expect(tabNode(container, id).dataset.tabEntering).toBeUndefined();
    }
  });

  it('an ACTIVE new tab swaps entering for entered when its entrance ends, until it stops being active', () => {
    const { container, rerender, props } = renderStrip([tab('a')], { activeTabId: 'a' });
    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} activeTabId="b" />);

    const b = tabNode(container, 'b');
    expect(b.dataset.tabEntering).toBe('true');
    expect(b.dataset.tabEntered).toBeUndefined();

    // The entrance ends. Dropping `entering` alone would swap the computed
    // animation to `br-tab-select` and settle the tab a second time, so the
    // active tab takes `entered` (computes `animation: none`) in the same tick.
    fireEvent.animationEnd(b);
    expect(b.dataset.tabEntering).toBeUndefined();
    expect(b.dataset.tabEntered).toBe('true');

    // Another tab becomes active: now it lets go, and selecting it later settles.
    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} activeTabId="a" />);
    expect(b.dataset.tabEntering).toBeUndefined();
    expect(b.dataset.tabEntered).toBeUndefined();
  });

  it('an INACTIVE new tab lets go of the attribute when its entrance ends', () => {
    const { container, rerender, props } = renderStrip([tab('a')], { activeTabId: 'a' });
    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} activeTabId="a" />);

    const b = tabNode(container, 'b');
    expect(b.dataset.tabEntering).toBe('true');
    // A bubbling animationend from a child (the running dot) is not its own.
    fireEvent.animationEnd(b.querySelector('button') as HTMLElement);
    expect(b.dataset.tabEntering).toBe('true');
    fireEvent.animationEnd(b);
    expect(b.dataset.tabEntering).toBeUndefined();
    // An inactive tab has no settle to suppress, so it takes no `entered` mark.
    expect(b.dataset.tabEntered).toBeUndefined();
  });

  it('marks the entrance under reduced motion too (the CSS rests it to a fade)', () => {
    window.matchMedia = ((query: string) =>
      ({
        matches: query === '(prefers-reduced-motion: reduce)',
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }) as unknown as MediaQueryList) as typeof window.matchMedia;

    const { container, rerender, props } = renderStrip([tab('a')]);
    rerender(<ChatTabStrip {...props} tabs={[tab('a'), tab('b')]} />);

    expect(tabNode(container, 'b').dataset.tabEntering).toBe('true');
    expect(frames.size).toBe(0);
  });

  it('while the window is being resized nothing enters and nothing slides', () => {
    document.body.classList.add('biorouter-window-resizing');
    try {
      const { container, rerender, props } = renderStrip([tab('a'), tab('b')]);
      rerender(<ChatTabStrip {...props} tabs={[tab('b'), tab('a'), tab('c')]} />);

      expect(tabNode(container, 'c').dataset.tabEntering).toBeUndefined();
      expect(tabNode(container, 'a').style.translate).toBe('');
      expect(tabNode(container, 'b').style.translate).toBe('');
      expect(frames.size).toBe(0);
    } finally {
      document.body.classList.remove('biorouter-window-resizing');
    }
  });

  it('the keyframe grows the tab from its left edge on the individual scale, and enter wins over select', () => {
    const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
    const rule = css.match(/\.br-tab\[data-tab-entering='true'\]\s*\{([^}]*)\}/);
    expect(rule, 'no .br-tab[data-tab-entering] rule').not.toBeNull();
    expect(rule![1]).toMatch(/transform-origin:\s*left center/);
    expect(rule![1]).toMatch(
      /animation:\s*br-tab-enter var\(--dur-fast-max\) var\(--ease-out\) backwards/
    );

    const keyframes = css.match(/@keyframes br-tab-enter\s*\{([\s\S]*?)\n\}/);
    expect(keyframes, 'no br-tab-enter keyframes').not.toBeNull();
    expect(keyframes![1]).toMatch(/opacity:\s*0;\s*scale:\s*0\.96;/);
    expect(keyframes![1]).not.toMatch(/transform:/);

    // Same specificity as the active rule, so source order decides: enter is later.
    expect(css.indexOf(".br-tab[data-tab-entering='true'] {")).toBeGreaterThan(
      css.indexOf(".br-tab[data-active='true'] {")
    );

    // An active tab whose entrance has played holds NO animation: not the settle,
    // and not a finished entrance a DOM move (React's keyed reorder) would replay.
    const entered = css.match(
      /\.br-tab\[data-active='true'\]\[data-tab-entered='true'\]\s*\{([^}]*)\}/
    );
    expect(entered, 'no .br-tab[data-tab-entered] rule').not.toBeNull();
    expect(entered![1]).toMatch(/animation:\s*none/);
  });
});
