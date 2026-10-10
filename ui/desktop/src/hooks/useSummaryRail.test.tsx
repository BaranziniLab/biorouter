import { act, render } from '@testing-library/react';
import { useRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  SUMMARY_RAIL_EXIT_FALLBACK_MS,
  SUMMARY_RAIL_SHOW_SETTLE_MS,
  useSummaryRail,
  type SummaryRailController,
  type UseSummaryRailOptions,
} from './useSummaryRail';
import { resetSummaryRailPreferenceForTests } from '../components/Layout/summaryRailPreference';

let observers: Array<() => void> = [];
const originalResizeObserver = globalThis.ResizeObserver;
const pane = { width: 1152 };

type HarnessProps = Omit<UseSummaryRailOptions, 'splitPaneRef'>;

let latest: SummaryRailController;
let commits: Array<Pick<SummaryRailController, 'rendered' | 'still' | 'shown'>> = [];

function Harness(props: HarnessProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const attach = (element: HTMLDivElement | null) => {
    if (element) {
      Object.defineProperty(element, 'clientWidth', {
        configurable: true,
        get: () => pane.width,
      });
    }
    ref.current = element;
  };
  const rail = useSummaryRail({ splitPaneRef: ref, ...props });
  latest = rail;
  commits.push({ rendered: rail.rendered, still: rail.still, shown: rail.shown });
  return <div ref={attach} {...rail.splitPaneProps} data-testid="split" />;
}

const defaults: HarnessProps = {
  active: true,
  isMobile: false,
  previewMode: null,
  previewWidth: 0,
};

function mount(over: Partial<HarnessProps> = {}) {
  const props = { ...defaults, ...over };
  const view = render(<Harness {...props} />);
  return {
    ...view,
    update: (next: Partial<HarnessProps>) => {
      Object.assign(props, next);
      view.rerender(<Harness {...props} />);
    },
  };
}

function resizeTo(width: number) {
  pane.width = width;
  act(() => {
    for (const callback of observers) callback();
  });
}

beforeEach(() => {
  observers = [];
  commits = [];
  pane.width = 1152;
  window.localStorage.clear();
  resetSummaryRailPreferenceForTests();
  class FakeResizeObserver {
    constructor(callback: () => void) {
      observers.push(callback);
    }
    observe() {}
    disconnect() {}
    unobserve() {}
  }
  globalThis.ResizeObserver = FakeResizeObserver as unknown as typeof ResizeObserver;
});

afterEach(() => {
  globalThis.ResizeObserver = originalResizeObserver;
  vi.useRealTimers();
  window.localStorage.clear();
  resetSummaryRailPreferenceForTests();
});

describe('useSummaryRail', () => {
  it('measures before the first paint: no observer callback and no timer needed', () => {
    vi.useFakeTimers();
    const { getByTestId } = mount();
    expect(latest.shown).toBe(true);
    expect(latest.width).toBe(280);
    expect(latest.mode).toBe('rail');
    const split = getByTestId('split');
    expect(split).toHaveAttribute('data-summary-rail', '');
    expect(split.style.getPropertyValue('--summary-rail-width')).toBe('280px');
  });

  it('appears at rest on mount: a tab switch or reload does not animate it', () => {
    const onGlide = vi.fn();
    mount({ onGlide });
    expect(commits.filter((c) => c.rendered).every((c) => c.still)).toBe(true);
    expect(onGlide).not.toHaveBeenCalled();
  });

  it('opens the popover instead where it does not fit', () => {
    pane.width = 1000;
    const { getByTestId } = mount();
    expect(latest.shown).toBe(false);
    expect(latest.mode).toBe('popover');
    expect(getByTestId('split')).not.toHaveAttribute('data-summary-rail');
  });

  it('hides at once when the pane narrows', () => {
    vi.useFakeTimers();
    mount();
    expect(latest.shown).toBe(true);
    resizeTo(1000);
    expect(latest.shown).toBe(false);
    expect(latest.rendered).toBe(false);
  });

  it('shows only after the width has held still for the settle delay', () => {
    vi.useFakeTimers();
    pane.width = 1000;
    mount();
    expect(latest.shown).toBe(false);
    resizeTo(1152);
    expect(latest.shown).toBe(false);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_SHOW_SETTLE_MS - 10));
    // Another sample restarts the settle.
    resizeTo(1160);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_SHOW_SETTLE_MS - 1));
    expect(latest.shown).toBe(false);
    act(() => vi.advanceTimersByTime(1));
    expect(latest.shown).toBe(true);
    // A geometry change appears at rest.
    expect(commits.filter((c) => c.rendered).every((c) => c.still)).toBe(true);
  });

  it('keeps the return buffer after a measured hide', () => {
    vi.useFakeTimers();
    mount();
    resizeTo(1055);
    expect(latest.shown).toBe(false);
    resizeTo(1067);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_SHOW_SETTLE_MS));
    expect(latest.shown).toBe(false);
    resizeTo(1068);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_SHOW_SETTLE_MS));
    expect(latest.shown).toBe(true);
  });

  it('hides in the same render a side preview takes the room, and returns when it closes', () => {
    const view = mount();
    expect(latest.shown).toBe(true);
    view.update({ previewMode: 'side', previewWidth: 512 });
    expect(latest.shown).toBe(false);
    view.update({ previewMode: null, previewWidth: 0 });
    expect(latest.shown).toBe(true);
    view.update({ previewMode: 'stack', previewWidth: 0 });
    expect(latest.shown).toBe(false);
  });

  it('stays hidden in an empty chat and on a phone-width browser', () => {
    const view = mount({ active: false });
    expect(latest.shown).toBe(false);
    expect(latest.mode).toBe('popover');
    view.update({ active: true, isMobile: true });
    expect(latest.shown).toBe(false);
    expect(latest.mode).toBe('popover');
  });

  it('animates when the first turn starts, and glides the conversation left by half the rail', () => {
    const onGlide = vi.fn();
    const view = mount({ active: false, onGlide });
    commits = [];
    view.update({ active: true });
    expect(latest.shown).toBe(true);
    // The commit that mounted the card carried `still: false`; the card latches it.
    expect(commits.some((c) => c.rendered && !c.still)).toBe(true);
    expect(onGlide).toHaveBeenCalledWith(140, 'open');
    // Only that one commit: later geometry changes are still again.
    expect(latest.still).toBe(true);
  });

  it('closes on a toggle: plays the exit, then drops the column and glides back', () => {
    const onGlide = vi.fn();
    mount({ onGlide });
    act(() => latest.toggle());
    expect(latest.preference).toBe('closed');
    expect(latest.shown).toBe(false);
    expect(latest.rendered).toBe(true);
    expect(latest.state).toBe('closed');
    expect(onGlide).not.toHaveBeenCalled();
    act(() => latest.onExited());
    expect(latest.rendered).toBe(false);
    expect(onGlide).toHaveBeenCalledWith(-140, 'close');
    // A second report changes nothing.
    act(() => latest.onExited());
    expect(onGlide).toHaveBeenCalledTimes(1);
  });

  it('drops the column after the exit fallback when the card never reports', () => {
    vi.useFakeTimers();
    mount();
    act(() => latest.toggle());
    expect(latest.rendered).toBe(true);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_EXIT_FALLBACK_MS));
    expect(latest.rendered).toBe(false);
  });

  it('opens on a toggle with its entrance and a glide', () => {
    window.localStorage.setItem('biorouter:summary-rail', 'closed');
    resetSummaryRailPreferenceForTests();
    const onGlide = vi.fn();
    mount({ onGlide });
    expect(latest.shown).toBe(false);
    expect(latest.mode).toBe('rail');
    commits = [];
    act(() => latest.toggle());
    expect(latest.shown).toBe(true);
    expect(latest.preference).toBe('open');
    expect(commits.some((c) => c.rendered && !c.still)).toBe(true);
    expect(onGlide).toHaveBeenCalledWith(140, 'open');
  });

  it('does not glide back for a rail the grid already dropped mid-exit', () => {
    vi.useFakeTimers();
    const onGlide = vi.fn();
    mount({ onGlide });
    act(() => latest.toggle());
    resizeTo(1000);
    expect(latest.rendered).toBe(false);
    act(() => latest.onExited());
    expect(onGlide).not.toHaveBeenCalled();
    // A later geometry change is not mistaken for the close.
    resizeTo(1152);
    act(() => vi.advanceTimersByTime(SUMMARY_RAIL_SHOW_SETTLE_MS));
    expect(onGlide).not.toHaveBeenCalled();
  });
});
