import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';
import { createTabStripMemory, type TabStripMemory } from './tabStripMemory';

function tab(over: Partial<ChatTab> = {}): ChatTab {
  return { tabId: 'tab-1', sessionId: 's1', title: 'Cohort query', userSetName: false, ...over };
}

const TABS = [
  tab(),
  tab({ tabId: 'tab-2', sessionId: 's2', title: 'Variant calls' }),
  tab({ tabId: 'tab-3', sessionId: 's3', title: 'Third' }),
];

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: TABS,
    activeTabId: 'tab-3',
    runningSessionIds: [],
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onReorder: vi.fn(),
    reserveTitlebar: false,
    isCompactSidebarOverlayOpen: false,
    ...over,
  };
  return { ...render(<ChatTabStrip {...props} />), props };
}

let resizeCallbacks: Array<() => void> = [];
const originalResizeObserver = globalThis.ResizeObserver;

beforeEach(() => {
  resizeCallbacks = [];
  class FakeResizeObserver {
    constructor(callback: () => void) {
      resizeCallbacks.push(callback);
    }
    observe() {}
    disconnect() {}
    unobserve() {}
  }
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver;
});

afterEach(() => {
  globalThis.ResizeObserver = originalResizeObserver;
});

/**
 * Tabs 117px wide every 120px from 0, the strip's 8px end padding after the last, and the
 * strip's 32px edge fade (chat-tabs.css, which jsdom does not load). jsdom lays nothing out and
 * never clamps scrollLeft, so the reveal's own clamp to [0, scrollWidth - clientWidth] is what
 * the numbers below read.
 */
function geometry(container: HTMLElement, width = 160) {
  const strip = container.querySelector('[role="tablist"]') as HTMLElement;
  const tabs = [...container.querySelectorAll<HTMLElement>('[data-tab-id]')];
  Object.defineProperty(strip, 'clientWidth', { configurable: true, value: width });
  Object.defineProperty(strip, 'scrollWidth', {
    configurable: true,
    value: (tabs.length - 1) * 120 + 117 + 8,
  });
  strip.style.setProperty('--tab-edge-fade', '32px');
  for (const [index, el] of tabs.entries()) {
    Object.defineProperty(el, 'offsetLeft', { configurable: true, value: index * 120 });
    Object.defineProperty(el, 'offsetWidth', { configurable: true, value: 117 });
  }
  return strip;
}
function resize() {
  act(() => {
    for (const callback of resizeCallbacks) callback();
  });
}
describe('the complete active tab is brought into view', () => {
  it('reveals the trailing edge including the close control after resize', () => {
    const { container } = renderStrip();
    const strip = geometry(container);
    resize();
    expect(strip.scrollLeft).toBe(205);
    expect(240 + 117).toBeLessThanOrEqual(strip.scrollLeft + strip.clientWidth - 8);
  });
  it('reveals the leading edge on selection', () => {
    const { container, rerender, props } = renderStrip();
    const strip = geometry(container);
    strip.scrollLeft = 205;
    rerender(<ChatTabStrip {...props} activeTabId="tab-1" />);
    expect(strip.scrollLeft).toBe(0);
    rerender(<ChatTabStrip {...props} />);
    expect(strip.scrollLeft).toBe(205);
  });
  it('reveals an already selected tab and responds to split-pane shrink', () => {
    const { container } = renderStrip();
    const strip = geometry(container, 100);
    fireEvent.click(container.querySelector('[data-tab-id="tab-3"] button[role="tab"]')!);
    expect(strip.style.getPropertyValue('--chat-tab-available-width')).toBe('92px');
    // 117px of tab in a 100px strip (jsdom ignores the 92px cap): some of it is out of the box
    // wherever it scrolls, so the reveal keeps its start, the glyph and the title, in view.
    expect(strip.scrollLeft).toBe(240);
    geometry(container, 200);
    resize();
    // The strip's end now: scrollWidth 365 - clientWidth 200. (jsdom never clamps the 265 a
    // 100px strip held, so the reveal's own clamp is what brings it back.)
    expect(strip.scrollLeft).toBe(165);
  });
  it('reveals the selected tab when keyboard focus returns to the strip', () => {
    const { container } = renderStrip();
    const strip = geometry(container);
    fireEvent.focus(container.querySelector('[data-tab-id="tab-3"] button[role="tab"]')!);
    expect(strip.scrollLeft).toBe(205);
  });

  it('shows the new selection after arrow-key navigation', () => {
    const { container, rerender, props } = renderStrip({ activeTabId: 'tab-2' });
    const strip = geometry(container);
    fireEvent.keyDown(container.querySelector('[data-tab-id="tab-2"] button[role="tab"]')!, {
      key: 'ArrowRight',
    });
    expect(props.onSelect).toHaveBeenCalledWith('tab-3');
    rerender(<ChatTabStrip {...props} activeTabId="tab-3" />);
    expect(strip.scrollLeft).toBe(205);
    expect(document.activeElement).toBe(
      container.querySelector('[data-tab-id="tab-3"] button[role="tab"]')
    );
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
    expect(props.onSelect).toHaveBeenLastCalledWith('tab-1');
    rerender(<ChatTabStrip {...props} activeTabId="tab-1" />);
    expect(document.activeElement).toBe(
      container.querySelector('[data-tab-id="tab-1"] button[role="tab"]')
    );
    expect(strip.scrollLeft).toBe(0);
  });

  it('keeps arrow-key focus through the strip remount caused by a chat switch', () => {
    const first = renderStrip({ activeTabId: 'tab-1' });
    fireEvent.keyDown(first.container.querySelector('[data-tab-id="tab-1"] button[role="tab"]')!, {
      key: 'ArrowRight',
    });
    first.unmount();
    const second = renderStrip({ activeTabId: 'tab-2' });
    expect(document.activeElement).toBe(
      second.container.querySelector('[data-tab-id="tab-2"] button[role="tab"]')
    );
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
    expect(second.props.onSelect).toHaveBeenLastCalledWith('tab-3');
    second.unmount();
    const third = renderStrip({ activeTabId: 'tab-3' });
    expect(document.activeElement).toBe(
      third.container.querySelector('[data-tab-id="tab-3"] button[role="tab"]')
    );
  });

  // Vision V1-R5-1: the strip's own scroll re-renders it (the edge fade's flags
  // flip), and a reveal on that render pulled every scroll back to the active tab.
  describe('a scroll the person made holds', () => {
    function scrollable(container: HTMLElement) {
      const strip = geometry(container);
      resize();
      expect(strip.scrollLeft).toBe(205);
      return strip;
    }
    function scrollTo(strip: HTMLElement, left: number) {
      act(() => {
        strip.scrollLeft = left;
        fireEvent.scroll(strip);
      });
    }

    it('through the re-render its own scroll causes', () => {
      const { container } = renderStrip();
      const strip = scrollable(container);
      expect(strip.hasAttribute('data-scrolled-start')).toBe(true);
      scrollTo(strip, 0);
      // The edge flags flipped, so the strip did re-render...
      expect(strip.hasAttribute('data-scrolled-start')).toBe(false);
      expect(strip.hasAttribute('data-scrolled-end')).toBe(true);
      // ...and the scroll is where the person left it.
      expect(strip.scrollLeft).toBe(0);
      scrollTo(strip, 100);
      expect(strip.scrollLeft).toBe(100);
    });

    it('through a parent re-render that moves no tab', () => {
      const { container, rerender, props } = renderStrip();
      const strip = scrollable(container);
      scrollTo(strip, 0);
      rerender(<ChatTabStrip {...props} runningSessionIds={['s1']} />);
      rerender(<ChatTabStrip {...props} runningSessionIds={[]} />);
      expect(strip.scrollLeft).toBe(0);
    });

    it('until something moves the active tab or the strip', () => {
      const { container, rerender, props } = renderStrip();
      const strip = scrollable(container);
      scrollTo(strip, 0);
      // A rename before the active tab moves it: revealed again.
      const active = container.querySelector<HTMLElement>('[data-tab-id="tab-3"]')!;
      Object.defineProperty(active, 'offsetLeft', { configurable: true, value: 260 });
      Object.defineProperty(strip, 'scrollWidth', { configurable: true, value: 385 });
      rerender(
        <ChatTabStrip
          {...props}
          tabs={[TABS[0], { ...TABS[1], title: 'Variant calls, chromosome 7' }, TABS[2]]}
        />
      );
      expect(strip.scrollLeft).toBe(225);
      // The strip narrowing (the ▾ appearing) reveals it too.
      scrollTo(strip, 0);
      Object.defineProperty(strip, 'clientWidth', { configurable: true, value: 140 });
      resize();
      expect(strip.scrollLeft).toBe(245);
    });
  });

  // Vision V2-R4-4: a tab revealed from an end stopped 8px from it, inside the 32px fade, so its
  // last 24px, the close slot and the end of its accent strip were faded out. Five tabs in a
  // 300px strip: tab-3 is 240..357, the strip's end is 605, so it scrolls 0..305.
  describe('a revealed tab clears the edge fade on the side it comes from', () => {
    const FIVE = [
      ...TABS,
      tab({ tabId: 'tab-4', sessionId: 's4', title: 'Fourth' }),
      tab({ tabId: 'tab-5', sessionId: 's5', title: 'Fifth' }),
    ];
    function five(activeTabId: string) {
      const rendered = renderStrip({ tabs: FIVE, activeTabId });
      const strip = geometry(rendered.container, 300);
      resize();
      return { ...rendered, strip };
    }
    const select = (container: HTMLElement, id: string) =>
      fireEvent.click(container.querySelector(`[data-tab-id="${id}"] button[role="tab"]`)!);

    it('a tab straddling the end stops 32px short of it, not 8px', () => {
      const { strip, rerender, props } = five('tab-1');
      expect(strip.scrollLeft).toBe(0);
      rerender(<ChatTabStrip {...props} activeTabId="tab-3" />);
      // Its right edge (357) at the end fade's inner edge: 89 + 300 - 32.
      expect(strip.scrollLeft).toBe(89);
    });

    it('a tab straddling the start stops 32px past it', () => {
      const { strip, rerender, props } = five('tab-1');
      act(() => {
        strip.scrollLeft = 150;
        fireEvent.scroll(strip);
      });
      rerender(<ChatTabStrip {...props} activeTabId="tab-2" />);
      // Its left edge (120) at the start fade's inner edge.
      expect(strip.scrollLeft).toBe(88);
    });

    it('the active tab inside the box but under the fade is revealed when clicked', () => {
      const { strip, container } = five('tab-3');
      expect(strip.scrollLeft).toBe(89);
      act(() => {
        strip.scrollLeft = 70;
        fireEvent.scroll(strip);
      });
      // 240..357 is inside 70..370, but the fade covers 338..370.
      select(container, 'tab-3');
      expect(strip.scrollLeft).toBe(89);
      // Shown clear already: a second click does not move the strip.
      select(container, 'tab-3');
      expect(strip.scrollLeft).toBe(89);
    });

    it('the last tab goes to the end, where the strip stops fading', () => {
      const { strip } = five('tab-5');
      expect(strip.scrollLeft).toBe(305);
    });
  });

  // Vision V2-R7-1: the shell keys a tab's chat on the active tab, so every tab switch mounts a
  // new strip with the group's memory. Each new strip started at scroll 0 and the reveal then
  // carried the tab just clicked from under the pointer to the strip's end. jsdom lays nothing
  // out, so the geometry sits on the prototype, in place before the first effect runs: 14 tabs
  // 117px wide every 120px and an 8px end padding (scrollWidth 1685) in a 700px strip, which
  // scrolls 0..985. No fade is declared here, so the reveal reads 0.
  describe('a remount starts where the last mount left the scroll', () => {
    const FOURTEEN = Array.from({ length: 14 }, (_, i) =>
      tab({ tabId: `tab-${i + 1}`, sessionId: `s${i + 1}`, title: `Chat ${i + 1}` })
    );
    const indexOf = (el: HTMLElement) => Number(el.dataset.tabId?.slice('tab-'.length)) - 1;
    const isStrip = (el: HTMLElement) => el.getAttribute('role') === 'tablist';
    beforeEach(() => {
      vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
        this: HTMLElement
      ) {
        return isStrip(this) ? 700 : 0;
      });
      vi.spyOn(HTMLElement.prototype, 'scrollWidth', 'get').mockImplementation(function (
        this: HTMLElement
      ) {
        return isStrip(this) ? 13 * 120 + 117 + 8 : 0;
      });
      vi.spyOn(HTMLElement.prototype, 'offsetLeft', 'get').mockImplementation(function (
        this: HTMLElement
      ) {
        return this.dataset.tabId ? indexOf(this) * 120 : 0;
      });
      vi.spyOn(HTMLElement.prototype, 'offsetWidth', 'get').mockImplementation(function (
        this: HTMLElement
      ) {
        return this.dataset.tabId ? 117 : 0;
      });
    });
    afterEach(() => {
      vi.restoreAllMocks();
    });

    const mount = (memory: TabStripMemory, activeTabId: string) => {
      const rendered = renderStrip({ tabs: FOURTEEN, activeTabId, memory });
      const strip = rendered.container.querySelector<HTMLElement>('[role="tablist"]')!;
      return { ...rendered, strip };
    };

    it('keeps the remembered scroll when the new active tab is already in view', () => {
      const memory = createTabStripMemory();
      memory.scrollLeft = 985;
      // tab-10 (1080..1197) is inside 985..1685: nothing moves.
      const { strip } = mount(memory, 'tab-10');
      expect(strip.scrollLeft).toBe(985);
    });

    it('carries a scroll from one mount to the next, as a tab switch does', () => {
      const memory = createTabStripMemory();
      const first = mount(memory, 'tab-14');
      expect(first.strip.scrollLeft).toBe(985);
      act(() => {
        first.strip.scrollLeft = 400;
        fireEvent.scroll(first.strip);
      });
      expect(memory.scrollLeft).toBe(400);
      // 1685px of tabs in 700px: the ▾ is showing, and the next mount starts with it drawn.
      expect(memory.overflowMenu).toBe(true);
      // The person clicks tab-6 (600..717, inside 400..1100): the old strip goes and a new one
      // mounts with tab-6 active.
      first.unmount();
      expect(memory.scrollLeft).toBe(400);
      const second = mount(memory, 'tab-6');
      expect(second.strip.scrollLeft).toBe(400);
    });

    it('still reveals a new active tab that the remembered scroll does not show', () => {
      const memory = createTabStripMemory();
      memory.scrollLeft = 0;
      // "+" opened tab-14 at the end (1560..1677), out of 0..700.
      const { strip } = mount(memory, 'tab-14');
      expect(strip.scrollLeft).toBe(985);
    });
  });

  it('does not scroll the page or chat transcript to reveal a tab', () => {
    const spy = vi.spyOn(Element.prototype, 'scrollIntoView');
    const { container } = renderStrip();
    geometry(container);
    resize();
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});
