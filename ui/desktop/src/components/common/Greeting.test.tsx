import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Greeting, resetGreetingArrivalForTests, retainTabGreetings } from './Greeting';

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
  resetGreetingArrivalForTests();
});

describe('Greeting', () => {
  /**
   * ⚠ The heading's name must be the sentence. The per-character unroll this
   * replaced split the text into one element per letter, and the heading was
   * announced letter by letter until it carried an `aria-label`. One text node
   * needs neither the label nor a hidden copy.
   */
  it('is named by its sentence, read as one piece of text', () => {
    render(<Greeting />);
    const heading = screen.getByRole('heading');
    expect(heading.textContent).toMatch(/\?$/);
    expect(heading).toHaveAccessibleName(heading.textContent ?? '');
    expect(heading.querySelector('[aria-hidden]')).toBeNull();
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

  /**
   * Principle 9: a surface that mounts because of navigation appears at rest.
   * Home's greeting has no tab, so it fades on the window's first arrival and
   * is still on every return (the vision review measured Crew to Home
   * replaying it). Each return still draws its own sentence.
   */
  it('fades Home in once per window and is at rest on every return', () => {
    const first = render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(true);
    first.unmount();

    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(false);
    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(false);
  });

  it('keeps Home at rest after a reload, which keeps sessionStorage', () => {
    render(<Greeting />).unmount();
    expect(window.sessionStorage.getItem('biorouter-greeting-arrived')).not.toBeNull();
    // A reload re-evaluates the module: the stored flag is all that survives.
    resetGreetingArrivalForTests({ reload: true });
    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(false);
  });

  it('can be told not to move at all', () => {
    render(<Greeting animate={false} />);
    expect(animate).toHaveBeenLastCalledWith(false);
  });

  it('still fades when storage is blocked, once, on the module flag', () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked');
    });
    render(<Greeting />).unmount();
    expect(animate).toHaveBeenLastCalledWith(true);
    render(<Greeting />);
    expect(animate).toHaveBeenLastCalledWith(false);
    getItem.mockRestore();
    setItem.mockRestore();
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
