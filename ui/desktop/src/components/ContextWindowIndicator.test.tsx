import type React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ContextWindowGauge,
  ContextWindowIndicator,
  contextWindowModelKey,
} from './ContextWindowIndicator';
import { FOOTER_COPY } from './bottom_menu/copy';

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

  it('reads the bar as what is used, since the ring beside it reads what is left', () => {
    render(
      <ContextWindowGauge
        totalTokens={92_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(screen.getByText(FOOTER_COPY.gaugeUsed(72))).toBeInTheDocument();
    expect(screen.getByText('92k / 128k')).toBeInTheDocument();
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

  it('states the window in one tooltip line, the same words as its name', async () => {
    const user = userEvent.setup();
    render(
      <ContextWindowIndicator
        totalTokens={92_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );

    const line = FOOTER_COPY.contextTooltip(28, '92k', '128k');
    expect(line).toBe('28% context left · 92k of 128k');
    const button = screen.getByRole('button', { name: line });
    await user.hover(button);
    await screen.findByRole('tooltip');
    const tooltip = document.querySelector<HTMLElement>('[data-slot="tooltip-content"]');
    expect(tooltip).toHaveTextContent(line);
    // One line: no stacked blocks.
    expect(tooltip!.querySelectorAll('.block')).toHaveLength(0);
  });

  it('reports itself expanded while its gauge is open, which the footer item styles on', async () => {
    const user = userEvent.setup();
    render(
      <ContextWindowIndicator
        totalTokens={10_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    const button = screen.getByTestId('context-window-indicator');
    expect(button).toHaveAttribute('aria-expanded', 'false');
    await user.click(button);
    // `pickers.css` follows `aria-expanded`: the tooltip around the trigger
    // overwrites `data-state`, so `[data-state='open']` never matched.
    expect(button).toHaveAttribute('aria-expanded', 'true');
  });

  it('draws the ring at the chip size with a 2px stroke, and no mono', () => {
    render(
      <ContextWindowIndicator
        totalTokens={10_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
        showRemainingPercent
      />
    );
    const button = screen.getByTestId('context-window-indicator');
    expect(button).toHaveClass('br-footline__item');
    const circles = button.querySelectorAll('circle');
    expect(circles).toHaveLength(2);
    circles.forEach((circle) => expect(circle).toHaveAttribute('stroke-width', '2'));
    expect(button.querySelector('.font-mono')).toBeNull();
  });

  it.each([
    [10_000, false],
    [64_000, false],
    [65_000, true],
    [120_000, true],
  ])('with %i of 128k used, prints the figure: %s', (used, shown) => {
    render(
      <ContextWindowIndicator
        totalTokens={used}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
        showRemainingPercent
      />
    );
    const remaining = Math.round(((128_000 - used) / 128_000) * 100);
    const figure = screen.queryByText(FOOTER_COPY.contextLeft(remaining));
    if (shown) expect(figure).toBeInTheDocument();
    else expect(figure).toBeNull();
  });

  it('never prints the figure unless asked to', () => {
    render(
      <ContextWindowIndicator
        totalTokens={120_000}
        tokenLimit={128_000}
        isTokenLimitLoaded
        onCompact={vi.fn()}
      />
    );
    expect(screen.queryByText(/% left$/)).toBeNull();
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
    expect(screen.queryByText(/context left/)).toBeNull();
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
      screen.getByRole('button', { name: '100% context left · 0 of 128k' })
    ).toBeInTheDocument();
  });
});

/**
 * V2-R3-5. The arc sweeps (250ms) only for a live update in the same chat. A
 * reading that arrives with the chat (the count and the window land about a
 * second after the composer mounts on an open or a tab switch), a change of
 * chat, and the window re-read after a model change all land at rest: the render
 * that moves the arc carries `data-motion-still`, which `pickers.css` turns into
 * a 0ms transition. Chromium proves the transition itself in
 * `bottom_menu/contextRingMotion.browser.test.tsx`.
 */
describe('the ring arc moves only for a live update in the same chat (V2-R3-5)', () => {
  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="chat-a"
      totalTokens={10_000}
      tokenLimit={128_000}
      isTokenLimitLoaded
      onCompact={vi.fn()}
      {...props}
    />
  );
  const arc = () => screen.getByTestId('context-ring-arc');

  it('draws its first reading at rest', () => {
    render(ring({}));
    expect(arc()).toHaveClass('br-footline__ring-arc');
    // A new element has nothing to transition from; no flag needed.
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });

  it('lands the token count that arrives after an open at rest', () => {
    const { rerender } = render(ring({ totalTokens: undefined }));
    rerender(ring({ totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands the window that arrives after the count at rest', () => {
    const { rerender } = render(ring({ totalTokens: 92_000, isTokenLimitLoaded: false }));
    // Nothing is drawn against the fallback while the window loads (V2-R4-1).
    expect(screen.queryByTestId('context-ring-arc')).toBeNull();
    rerender(ring({ totalTokens: 92_000, tokenLimit: 400_000, isTokenLimitLoaded: true }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands a re-read window at rest after holding the last one', () => {
    const { rerender } = render(ring({ totalTokens: 92_000, tokenLimit: 1_000_000 }));
    rerender(ring({ totalTokens: 92_000, tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    rerender(ring({ totalTokens: 92_000, tokenLimit: 200_000, isTokenLimitLoaded: true }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('lands another chat’s reading at rest when the composer changes chat', () => {
    const { rerender } = render(ring({ totalTokens: 10_000 }));
    rerender(ring({ sessionId: 'chat-b', totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
  });

  it('sweeps for a live update in the same chat', () => {
    const { rerender } = render(ring({ totalTokens: 10_000 }));
    rerender(ring({ totalTokens: 30_000 }));
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });

  it('releases the flag after the still frame, so the next live update sweeps', async () => {
    const { rerender } = render(ring({ totalTokens: undefined }));
    rerender(ring({ totalTokens: 92_000 }));
    expect(arc()).toHaveAttribute('data-motion-still');
    await waitFor(() => expect(arc()).not.toHaveAttribute('data-motion-still'));
    rerender(ring({ totalTokens: 100_000 }));
    expect(arc()).not.toHaveAttribute('data-motion-still');
  });
});

/**
 * V2-R4-1. The composer mounts with a 128k fallback in `tokenLimit` and
 * `isTokenLimitLoaded` false, and the chat's real window lands about a second
 * later. Measured on a tab switch: a 1M chat 10% full read "20% context left ·
 * 102.4k of 128k", and a 1M chat 18% full drew the empty ring and "0% left" for
 * 700ms. No ring and no figure until a window has loaded for the chat; a re-read
 * in the same chat holds the last loaded window instead of blinking out.
 */
describe('the ring draws only against a loaded window (V2-R4-1)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="chat-a"
      totalTokens={180_000}
      tokenLimit={128_000}
      isTokenLimitLoaded={false}
      onCompact={vi.fn()}
      showRemainingPercent
      {...props}
    />
  );

  /** The reading the ring announces, or none when nothing is drawn. */
  const reading = () =>
    screen.queryByTestId('context-window-indicator')?.getAttribute('aria-label') ?? null;

  it('draws no ring and no figure while the window loads, then one reading', () => {
    const { container, rerender } = render(ring({}));
    // 180k of the 128k fallback would read "0% left" beside an empty ring.
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText(/% left$/)).toBeNull();

    rerender(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(82, '180k', '1M'));
    // 82% left is above the figure's threshold: the ring says it alone.
    expect(screen.queryByText(/% left$/)).toBeNull();
    const circumference = 2 * Math.PI * 6;
    const offset = Number(screen.getByTestId('context-ring-arc').getAttribute('stroke-dashoffset'));
    expect(offset).toBeCloseTo(circumference * 0.18, 6);
  });

  it('draws nothing while the window loads, also where the figure would print', () => {
    // 100k of the fallback is 22% left and would print "22% left".
    const { container } = render(ring({ totalTokens: 100_000 }));
    expect(container).toBeEmptyDOMElement();
  });

  it('holds the last loaded window while the same chat re-reads it', () => {
    const { rerender } = render(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true }));
    const loaded = FOOTER_COPY.contextTooltip(82, '180k', '1M');
    expect(reading()).toBe(loaded);
    // The model changed: the composer marks the window unloaded and keeps the old
    // value until the lookup lands. The ring keeps its reading, no blink.
    rerender(ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(loaded);
    // Even if the composer put its fallback back while loading, the ring holds the
    // window it loaded.
    rerender(ring({ tokenLimit: 128_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(loaded);
    rerender(ring({ tokenLimit: 200_000, isTokenLimitLoaded: true }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(10, '180k', '200k'));
    expect(screen.getByText(FOOTER_COPY.contextLeft(10))).toBeInTheDocument();
  });

  it('holds nothing across a change of chat', () => {
    const { container, rerender } = render(
      ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true })
    );
    rerender(ring({ sessionId: 'chat-b', tokenLimit: 1_000_000, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();
  });

  it('holds nothing when the re-read finds no model', () => {
    const { container, rerender } = render(
      ring({ tokenLimit: 1_000_000, isTokenLimitLoaded: true })
    );
    rerender(ring({ tokenLimit: 0, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();
  });

  it('the gauge alone draws nothing against a window that has not loaded', () => {
    const { container } = render(
      <ContextWindowGauge
        totalTokens={180_000}
        tokenLimit={128_000}
        isTokenLimitLoaded={false}
        onCompact={vi.fn()}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });
});

/**
 * V2-R4-1, round 5. A chat opened for the first time (a tab restored after a
 * reload, a hash or History open) loads for up to seconds, and until its row
 * lands the composer has no binding: it looks up the window of the model for NEW
 * chats, and that lookup is loaded. Measured on a 200k chat: "0 of 1.1M", then a
 * sweep to "64.6k of 1.1M", then "of 200k" five seconds later. The ring now draws
 * a window only when it was looked up for the model the chat is bound to
 * (`tokenLimitModel === boundModel`), and nothing while that model is unknown.
 */
describe('the ring draws only a window looked up for the chat’s own model (V2-R4-1, round 5)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.read.mockResolvedValue(null);
    mocks.upsert.mockResolvedValue(undefined);
  });

  const NEW_CHAT_MODEL = 'versa_azure/gpt-5.5';
  const CHAT_MODEL = 'anthropic/claude-opus-4-5';

  const ring = (props: Partial<React.ComponentProps<typeof ContextWindowIndicator>>) => (
    <ContextWindowIndicator
      sessionId="demo2"
      totalTokens={0}
      tokenLimit={1_050_000}
      isTokenLimitLoaded
      tokenLimitModel={NEW_CHAT_MODEL}
      boundModel={null}
      onCompact={vi.fn()}
      showRemainingPercent
      {...props}
    />
  );
  const reading = () =>
    screen.queryByTestId('context-window-indicator')?.getAttribute('aria-label') ?? null;

  it('draws nothing while the chat loads, then one reading at rest for its own model', () => {
    // Loading: the binding is unknown and the window loaded is the new-chat model's.
    const { container, rerender } = render(ring({}));
    expect(container).toBeEmptyDOMElement();

    // The row lands: the chat's tokens and its binding arrive in one render, while
    // the composer still holds the new-chat model's window (its re-read starts in
    // an effect after this render).
    rerender(ring({ totalTokens: 64_600, boundModel: CHAT_MODEL }));
    expect(container).toBeEmptyDOMElement();
    // The re-read starts: unloaded, the old value still in place.
    rerender(ring({ totalTokens: 64_600, boundModel: CHAT_MODEL, isTokenLimitLoaded: false }));
    expect(container).toBeEmptyDOMElement();

    // The chat's own window lands: one reading, at rest.
    rerender(
      ring({
        totalTokens: 64_600,
        boundModel: CHAT_MODEL,
        tokenLimitModel: CHAT_MODEL,
        tokenLimit: 200_000,
      })
    );
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(68, '64.6k', '200k'));
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('draws the loaded window at once when the chat turns out to use the new-chat model', () => {
    const { container, rerender } = render(ring({}));
    expect(container).toBeEmptyDOMElement();
    rerender(ring({ totalTokens: 110_100, boundModel: NEW_CHAT_MODEL }));
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(90, '110.1k', '1.1M'));
    // The first reading after the load, so nothing to sweep from.
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('holds the window across a re-read of the same model, and not across a model change', () => {
    const loaded = { totalTokens: 64_600, boundModel: CHAT_MODEL, tokenLimitModel: CHAT_MODEL };
    const { container, rerender } = render(ring({ ...loaded, tokenLimit: 200_000 }));
    const reading200k = FOOTER_COPY.contextTooltip(68, '64.6k', '200k');
    expect(reading()).toBe(reading200k);

    // The app's selection moved under this pinned chat: the lookup re-runs for the
    // same model. The ring keeps its reading, no blink.
    rerender(ring({ ...loaded, tokenLimit: 200_000, isTokenLimitLoaded: false }));
    expect(reading()).toBe(reading200k);
    rerender(ring({ ...loaded, tokenLimit: 200_000 }));
    expect(reading()).toBe(reading200k);

    // The person picks another model for this chat: the old window is not this
    // model's, loaded or held, so nothing is drawn until the new one lands.
    rerender(ring({ ...loaded, boundModel: NEW_CHAT_MODEL, tokenLimit: 200_000 }));
    expect(container).toBeEmptyDOMElement();
    rerender(
      ring({
        ...loaded,
        boundModel: NEW_CHAT_MODEL,
        tokenLimit: 200_000,
        isTokenLimitLoaded: false,
      })
    );
    expect(container).toBeEmptyDOMElement();
    rerender(
      ring({
        ...loaded,
        boundModel: NEW_CHAT_MODEL,
        tokenLimitModel: NEW_CHAT_MODEL,
        tokenLimit: 1_050_000,
      })
    );
    expect(reading()).toBe(FOOTER_COPY.contextTooltip(94, '64.6k', '1.1M'));
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  it('drops the ring while a chat it was drawn for loads again', () => {
    const { container, rerender } = render(
      ring({ totalTokens: 64_600, boundModel: CHAT_MODEL, tokenLimitModel: CHAT_MODEL })
    );
    expect(reading()).not.toBeNull();
    rerender(ring({ totalTokens: 64_600, boundModel: null, tokenLimitModel: CHAT_MODEL }));
    expect(container).toBeEmptyDOMElement();
  });
});

describe('contextWindowModelKey', () => {
  it('names the chat’s binding when it has one, else the app’s selection', () => {
    expect(
      contextWindowModelKey({ provider: 'anthropic', model: 'claude-opus-4-5' }, 'versa_azure', 'x')
    ).toBe('anthropic/claude-opus-4-5');
    expect(contextWindowModelKey(undefined, 'versa_azure', 'gpt-5.5')).toBe('versa_azure/gpt-5.5');
    expect(contextWindowModelKey(undefined, null, undefined)).toBe('/');
  });
});
