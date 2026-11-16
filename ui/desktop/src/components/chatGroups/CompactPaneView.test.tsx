import { fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { CompactPaneView, type CompactPaneViewProps } from './CompactPaneView';
import { compactPaneCopy } from './copy';
import { DICTATION_LABELS } from '../dictation/copy';

/**
 * The compact pane's placeholder, drawn from its props (rung 5). One quiet button is the whole
 * placeholder; its words come from `copy.ts`; it says which chat it holds, whether that chat needs
 * the person, and how to get it back, to the eye and to assistive tech alike. Geometry and colour
 * are held in Chromium by `compactPane.browser.test.tsx`.
 */

function view(over: Partial<CompactPaneViewProps> = {}) {
  const props: CompactPaneViewProps = {
    title: 'Cohort query',
    kindSource: { name: 'Cohort query' },
    tier: 'public',
    isActiveGroup: false,
    state: 'idle',
    live: false,
    shortfall: 'width',
    density: 'card',
    band: { insetStart: 0, action: false, menu: null },
    action: { kind: 'grow', label: compactPaneCopy.grow.width, run: vi.fn() },
    announcement: '',
    animateIn: false,
    ...over,
  };
  return { ...render(<CompactPaneView {...props} />), props };
}

const actions = (root: HTMLElement) =>
  root.querySelectorAll<HTMLElement>('[data-compact-pane-action]');

describe('CompactPaneView', () => {
  it('says what the pane is short of, from copy.ts', () => {
    for (const shortfall of ['width', 'height', 'both'] as const) {
      const { container, unmount } = view({
        shortfall,
        action: { kind: 'grow', label: compactPaneCopy.grow[shortfall], run: () => {} },
      });
      expect(container.querySelector('.br-compact-pane__state')!.textContent).toBe(
        compactPaneCopy.shortfall[shortfall]
      );
      expect(container.querySelector('.br-compact-pane__verb')!.textContent).toBe(
        compactPaneCopy.grow[shortfall]
      );
      unmount();
    }
    expect(compactPaneCopy.shortfall.width).toBe('Too narrow to show');
    expect(compactPaneCopy.grow).toEqual({
      width: 'Make wider',
      height: 'Make taller',
      both: 'Make larger',
    });
    expect(compactPaneCopy.merge).toBe('Merge panes');
  });

  it('shows the most urgent state, in the one state line', () => {
    const cases = [
      ['running', compactPaneCopy.running],
      ['waiting', compactPaneCopy.waiting],
      ['listening', DICTATION_LABELS.listening],
    ] as const;
    for (const [state, words] of cases) {
      const { container, unmount } = view({ state, live: state !== 'listening' });
      const line = container.querySelector('.br-compact-pane__state')!;
      expect(line.textContent).toBe(words);
      expect(line.getAttribute('data-state')).toBe(state);
      unmount();
    }
    expect(compactPaneCopy.running).toBe('Running');
    expect(compactPaneCopy.waiting).toBe('Waiting for you');
  });

  it('follows the density prop', () => {
    for (const density of ['card', 'glyph', 'band'] as const) {
      const { container, unmount } = view({
        density,
        band: { insetStart: 0, action: density === 'band', menu: null },
      });
      expect(container.querySelector('.br-compact-pane')!.getAttribute('data-density')).toBe(
        density
      );
      unmount();
    }
  });

  it('has exactly one action in each density', () => {
    for (const density of ['card', 'glyph', 'band'] as const) {
      const { container, unmount } = view({
        density,
        band: { insetStart: 0, action: density === 'band', menu: null },
      });
      expect(actions(container), density).toHaveLength(1);
      const testId = actions(container)[0].getAttribute('data-testid');
      expect(testId).toBe(density === 'band' ? 'compact-pane-band-action' : 'compact-pane-action');
      unmount();
    }
  });

  it('keeps the same action node across card and glyph, so focus never moves', () => {
    const { container, rerender, props } = view({ density: 'card' });
    const before = actions(container)[0];
    rerender(<CompactPaneView {...props} density="glyph" />);
    expect(actions(container)[0]).toBe(before);
  });

  it('names the action by its verb and the title, and describes it by kind, state and help', () => {
    const { container } = view({ tier: 'private', state: 'running', live: true });
    const button = actions(container)[0];
    expect(button.getAttribute('aria-label')).toBe(
      compactPaneCopy.actionName(compactPaneCopy.grow.width, 'Cohort query')
    );
    expect(button.getAttribute('aria-label')).toBe('Make wider, Cohort query');
    const described = document.getElementById(button.getAttribute('aria-describedby')!)!;
    expect(described.textContent).toBe(
      `Private chat. Running. ${compactPaneCopy.help('width', true)}`
    );
    expect(compactPaneCopy.help('width', true)).toBe(
      'The chat is hidden, not closed. It comes back as it was when the pane is wider.'
    );
  });

  it('says merge in the help when the layout has no room', () => {
    const { container } = view({
      action: { kind: 'merge', label: compactPaneCopy.merge, run: () => {} },
    });
    const button = actions(container)[0];
    expect(button.getAttribute('aria-label')).toBe('Merge panes, Cohort query');
    const described = document.getElementById(button.getAttribute('aria-describedby')!)!;
    expect(described.textContent).toContain(compactPaneCopy.help('width', false));
  });

  it('runs the action on press', () => {
    const { container, props } = view();
    actions(container)[0].click();
    expect(props.action.run).toHaveBeenCalledTimes(1);
  });

  it('draws the lock badge for a private chat, and never Public for an unknown tier', () => {
    const privateView = view({ tier: 'private' });
    expect(
      privateView.container
        .querySelector('[data-testid="chat-kind-icon"]')!
        .getAttribute('data-privacy')
    ).toBe('private');
    privateView.unmount();
    const unknown = view({ tier: undefined });
    const glyph = unknown.container.querySelector('[data-testid="chat-kind-icon"]')!;
    expect(glyph.getAttribute('data-privacy')).toBe('unknown');
    expect(glyph.getAttribute('data-privacy')).not.toBe('public');
  });

  it('draws the running dot while live, hidden from assistive tech', () => {
    const idle = view({ live: false });
    expect(idle.container.querySelector('.br-compact-pane__dot')).toBeNull();
    idle.unmount();
    const { container } = view({ live: true, state: 'running' });
    const dot = container.querySelector('.br-compact-pane__dot')!;
    expect(dot).not.toBeNull();
    expect(dot.classList.contains('br-tab__dot')).toBe(true);
    expect(dot.getAttribute('aria-hidden')).toBe('true');
  });

  it('is a group named by the title, with one polite status line', () => {
    const { container } = view({ announcement: 'Cohort query is waiting for you' });
    const root = container.querySelector('.br-compact-pane')!;
    expect(root.getAttribute('role')).toBe('group');
    expect(root.getAttribute('aria-label')).toBe('Cohort query');
    const status = container.querySelectorAll('[role="status"]');
    expect(status).toHaveLength(1);
    expect(status[0].getAttribute('aria-live')).toBe('polite');
    expect(status[0].textContent).toBe('Cohort query is waiting for you');
  });

  it('fades in only when asked, and drops the mark when its own fade ends', () => {
    const rest = view({ animateIn: false });
    expect(rest.container.querySelector('.br-compact-pane')!.hasAttribute('data-enter')).toBe(
      false
    );
    rest.unmount();
    const { container } = view({ animateIn: true });
    const root = container.querySelector('.br-compact-pane')!;
    expect(root.hasAttribute('data-enter')).toBe(true);
    expect(root.hasAttribute('data-motion-layout')).toBe(true);
    fireEvent.animationEnd(root);
    expect(root.hasAttribute('data-enter')).toBe(false);
  });

  it('declares no drag app-region on its band', () => {
    const { container } = view();
    const band = container.querySelector<HTMLElement>('.br-compact-pane__band')!;
    expect(band.style.getPropertyValue('-webkit-app-region')).toBe('');
    expect(band.getAttribute('style') ?? '').not.toMatch(/app-region/);
  });
});
