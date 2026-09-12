import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, fireEvent, act } from '@testing-library/react';
import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';

/**
 * Defect 3.2. Resizing the window scrolled the ACTIVE tab off-screen and
 * nothing brought it back.
 *
 * The strip is a CSS scroll box; the browser preserves `scrollLeft` while both
 * `clientWidth` and the left gutter change under it. The only `scrollIntoView`
 * in the file was an effect keyed on `[activeTabId, tabs.length]` — neither of
 * which a resize touches — and the documented escape hatch, the ▾ overflow
 * menu, calls `onSelect(tabId)` with the comment "selecting scrolls it into
 * view through the effect above". For the tab that is ALREADY active, that
 * effect's deps are unchanged, so the one offered way back was a no-op for
 * exactly the tab that was off-screen.
 *
 * ⚠ Nothing here claims the tab is VISIBLE — jsdom computes no layout and has
 * no `scrollIntoView` at all (the production call is feature-detected for that
 * reason). What is asserted is that the reveal is REQUESTED on the active tab,
 * on each of the two paths that previously requested nothing. Whether it lands
 * is on the live checklist.
 */
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
let scrollIntoView: ReturnType<typeof vi.fn>;
const originalResizeObserver = globalThis.ResizeObserver;

beforeEach(() => {
  resizeCallbacks = [];
  scrollIntoView = vi.fn();
  (Element.prototype as unknown as { scrollIntoView: unknown }).scrollIntoView = scrollIntoView;
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
  delete (Element.prototype as unknown as { scrollIntoView?: unknown }).scrollIntoView;
});

describe('the active tab is brought back into view', () => {
  it('after the strip is resized', () => {
    const { container } = renderStrip();
    scrollIntoView.mockClear();

    act(() => {
      for (const callback of resizeCallbacks) callback();
    });

    expect(scrollIntoView).toHaveBeenCalled();
    // The ref sits on the tab's LABEL BUTTON, not on `.br-tab` — the wrapper
    // owns the drag gesture and nothing new may be declared on it.
    const active = container.querySelector('[data-tab-id="tab-3"] button[role="tab"]');
    expect(scrollIntoView.mock.instances).toContain(active);
  });

  it('when the overflow menu selects the tab that is already active', async () => {
    const { container } = renderStrip();
    scrollIntoView.mockClear();

    // The ▾ menu and the tab itself share one `handleSelect`; the ▾ is gated on
    // a measurement jsdom cannot make, so the shared handler is exercised
    // through the tab. Selecting the tab you are ALREADY on is the case the
    // effect's deps cannot see, and the case the ▾ exists to serve.
    const activeTabButton = container.querySelector(
      '[data-tab-id="tab-3"] button[role="tab"]'
    ) as HTMLElement;
    fireEvent.click(activeTabButton);

    expect(scrollIntoView.mock.instances).toContain(activeTabButton);
  });
});
