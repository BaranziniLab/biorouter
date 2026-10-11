import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';

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

function geometry(container: HTMLElement, width = 160) {
  const strip = container.querySelector('[role="tablist"]') as HTMLElement;
  Object.defineProperty(strip, 'clientWidth', { configurable: true, value: width });
  for (const [index, el] of [
    ...container.querySelectorAll<HTMLElement>('[data-tab-id]'),
  ].entries()) {
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
    expect(strip.scrollLeft).toBe(265);
    geometry(container, 200);
    resize();
    expect(strip.scrollLeft).toBe(240);
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
      Object.defineProperty(strip, 'scrollWidth', { configurable: true, value: 357 });
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

  it('does not scroll the page or chat transcript to reveal a tab', () => {
    const spy = vi.spyOn(Element.prototype, 'scrollIntoView');
    const { container } = renderStrip();
    geometry(container);
    resize();
    expect(spy).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});
