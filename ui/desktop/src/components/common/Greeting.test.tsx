import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Greeting, retainTabGreetings } from './Greeting';

const animate = vi.fn();
vi.mock('../../hooks/use-text-animator', () => ({
  useTextAnimator: (opts: { text: string; enabled?: boolean }) => {
    animate(opts.enabled !== false);
    return { current: null };
  },
}));

beforeEach(() => {
  animate.mockClear();
  retainTabGreetings([]);
});

describe('Greeting', () => {
  /**
   * ⚠ The unroll splits the sentence into one element PER CHARACTER, and
   * `split-type` emits no ARIA at all. Without a name on the heading itself, the
   * app's only orienting heading on an empty chat is announced letter by letter
   * - "W h a t   i n s i g h t s …" - and heading navigation is broken with it.
   * Everyone who has not turned on reduced motion gets that on every arrival.
   */
  it('keeps a readable accessible name, which the per-character split destroys', () => {
    render(<Greeting />);
    const heading = screen.getByRole('heading');
    const label = heading.getAttribute('aria-label');
    expect(label).toMatch(/\?$/);
    expect(label).toBe(heading.textContent);
    // And the split text must be hidden, or it is read as the content anyway.
    expect(heading.querySelector('span')?.getAttribute('aria-hidden')).toBe('true');
  });

  it('renders one of the stock sentences', () => {
    render(<Greeting />);
    expect(screen.getByRole('heading').textContent).toMatch(/\?$/);
  });

  /**
   * ⚠ The rotation is deliberate product voice. It was removed once as
   * marketing register and restored on the operator's instruction: a different
   * line on each arrival is the intent, so a change that collapses this to one
   * fixed sentence should fail here rather than pass quietly.
   */
  it('draws a different sentence across arrivals', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 40; i++) {
      const { container, unmount } = render(<Greeting />);
      seen.add(container.textContent ?? '');
      unmount();
    }
    expect(seen.size).toBeGreaterThan(1);
  });

  it('animates separate arrivals without a tab identity, and supports disabling motion', () => {
    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(true);

    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(true);

    render(<Greeting animate={false} />);
    expect(animate).toHaveBeenLastCalledWith(false);
  });

  it('keeps its sentence stable across a re-render of the same instance', () => {
    // A re-render must not swap the text out from under a running animation.
    const { container, rerender } = render(<Greeting />);
    const first = container.textContent;
    rerender(<Greeting />);
    expect(container.textContent).toBe(first);
  });
});

describe('tab greeting lifetime', () => {
  it('keeps the sentence and skips animation when the same tab moves to another pane', () => {
    const first = render(<Greeting tabId="tab-a" />);
    const message = first.container.textContent;
    expect(animate).toHaveBeenLastCalledWith(true);
    first.unmount();
    const moved = render(<Greeting tabId="tab-a" />);
    expect(moved.container.textContent).toBe(message);
    expect(animate).toHaveBeenLastCalledWith(false);
  });
  it('does not replay after visiting another tab or adding one', () => {
    const view = render(<Greeting key="a" tabId="a" />);
    const message = view.container.textContent;
    view.rerender(<Greeting key="b" tabId="b" />);
    expect(animate).toHaveBeenLastCalledWith(true);
    retainTabGreetings(['a', 'b']);
    view.rerender(<Greeting key="a" tabId="a" />);
    expect(view.container.textContent).toBe(message);
    expect(animate).toHaveBeenLastCalledWith(false);
  });
  it('releases closed tabs so a new lifetime gets a fresh greeting', () => {
    const random = vi.spyOn(Math, 'random').mockReturnValue(0);
    const first = render(<Greeting tabId="tab-a" />);
    const message = first.container.textContent;
    first.unmount();
    retainTabGreetings([]);
    random.mockReturnValue(0.99);
    const reopened = render(<Greeting tabId="tab-a" />);
    expect(reopened.container.textContent).not.toBe(message);
    expect(animate).toHaveBeenLastCalledWith(true);
    random.mockRestore();
  });
});
