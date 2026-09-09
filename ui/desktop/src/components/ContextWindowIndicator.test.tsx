import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { ContextWindowGauge, ContextWindowIndicator } from './ContextWindowIndicator';

// The gauge reads and writes the auto-compact threshold through ConfigContext
// (never straight to the API — see ContextWindowIndicator.configCache.test.tsx,
// which drives the real provider and asserts the cache stays consistent).
const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  upsert: vi.fn(),
}));

vi.mock('./ConfigContext', () => ({
  useConfig: () => ({ read: mocks.read, upsert: mocks.upsert }),
}));

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe('ContextWindowGauge compaction control', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  it('uses an inward compression icon and a Biorouter tooltip when disabled', async () => {
    const user = userEvent.setup();
    render(
      <ContextWindowGauge
        totalTokens={0}
        tokenLimit={1_100_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );

    const button = screen.getByRole('button', { name: 'Nothing to compact yet' });
    expect(button).toBeDisabled();
    expect(button).not.toHaveAttribute('title');
    expect(screen.getByTestId('compact-conversation-icon')).toHaveClass('size-4');
    // 1.5, not 1.75: design.md §3.9 mandates a single stroke weight for every
    // glyph. This used to assert 1.75 — lucide's native default, which leaked in
    // because the icon was imported straight from `lucide-react` instead of the
    // `light()` wrapper in app-icons. The test was pinning the drift (three
    // stroke weights rendered on one screen), so it moved with the fix.
    expect(screen.getByTestId('compact-conversation-icon')).toHaveAttribute('stroke-width', '1.5');

    await user.hover(button.parentElement!);
    const tooltip = await screen.findByRole('tooltip');
    expect(tooltip).toHaveTextContent('Nothing to compact yet');
    expect(document.querySelector('[data-slot="tooltip-content"]')).toHaveClass(
      'bg-background-inverse',
      'text-text-inverse',
      // `rounded-container` (12px). This asserted `rounded-sm`, then
      // `rounded-inner` — both 4px, the rung the ladder reserves for things
      // nested INSIDE a control. A tooltip is a floating surface, and the
      // cohesion design puts every floating surface on one 12px recipe; the
      // v1.89.0 review caught the tooltip as the last one still two steps off.
      'rounded-container'
    );
    expect(document.querySelector('[data-slot="tooltip-content"] svg')).toBeNull();
  });

  it('runs compaction from the enabled control', async () => {
    const user = userEvent.setup();
    const onCompact = vi.fn();
    render(
      <ContextWindowGauge
        totalTokens={24_000}
        tokenLimit={1_100_000}
        isTokenLimitLoaded
        onCompact={onCompact}
      />
    );

    const button = screen.getByRole('button', { name: 'Compact chat' });
    await user.click(button);
    expect(onCompact).toHaveBeenCalledTimes(1);
  });

  it('uses a compact multiline context tooltip', async () => {
    const user = userEvent.setup();
    render(
      <ContextWindowIndicator
        totalTokens={0}
        tokenLimit={1_100_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );

    const button = screen.getByRole('button', {
      name: 'Context window usage. 1.1M of 1.1M tokens remaining. 100% remaining, 0% used',
    });
    await user.hover(button);

    await screen.findByRole('tooltip');
    const tooltip = document.querySelector<HTMLElement>('[data-slot="tooltip-content"]');
    expect(tooltip).toHaveClass('w-52', 'text-left');
    const [titleLine, remainingLine, usageLine] = Array.from(tooltip!.children);
    expect(titleLine).toHaveClass('block', 'font-medium');
    expect(titleLine).toHaveTextContent('Context window usage');
    expect(remainingLine).toHaveClass('block');
    expect(remainingLine).toHaveTextContent('1.1M of 1.1M tokens remaining');
    expect(usageLine).toHaveClass('block');
    expect(usageLine).toHaveTextContent('100% remaining, 0% used');
  });
});

/**
 * F10. A context window is a property of a bound model; with no model there is
 * no window, and every figure this component could print would be about a model
 * that does not exist. Measured in onboarding: the composer's chip correctly
 * read "No model yet — choose a provider" while the gauge beside it read
 * "Context window usage. 128k of 128k tokens remaining".
 */
describe('with no model bound', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  it('renders nothing rather than a window for a model that does not exist', () => {
    const { container } = render(
      <ContextWindowIndicator
        totalTokens={0}
        tokenLimit={0}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/tokens remaining/)).toBeNull();
  });

  it('renders nothing in the popover-body gauge either', () => {
    const { container } = render(
      <ContextWindowGauge totalTokens={0} tokenLimit={0} isTokenLimitLoaded onCompact={vi.fn()} />
    );
    expect(container).toBeEmptyDOMElement();
  });

  /**
   * The guard is on the LIMIT, not only on the loaded flag: a lookup that
   * finished and found nothing is exactly the case, and usage without a window
   * is a pair nothing can render honestly either.
   */
  it('stays hidden even when tokens have been counted', () => {
    const { container } = render(
      <ContextWindowIndicator
        totalTokens={1766}
        tokenLimit={0}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('still renders once a real window is known', () => {
    render(
      <ContextWindowIndicator
        totalTokens={0}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(
      screen.getByRole('button', {
        name: 'Context window usage. 128k of 128k tokens remaining. 100% remaining, 0% used',
      })
    ).toBeInTheDocument();
  });
});
