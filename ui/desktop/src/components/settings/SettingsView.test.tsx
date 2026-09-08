import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import SettingsView from './SettingsView';
import { CONFIGURATION_ENABLED } from '../../updates';

// Every section but Privacy is stubbed, and each stub is identifiable, so this
// file can assert both that the Privacy panel is really mounted — the failure
// mode being a settings component that is declared and plausible but has zero
// consumers repo-wide, so it renders for nobody — and WHERE it sits relative to
// its neighbours.
vi.mock('./models/ModelsSection', () => ({ default: () => <div /> }));
vi.mock('./chat/ChatSettingsSection', () => ({ default: () => <div /> }));
vi.mock('./app/AppSettingsSection', () => ({
  default: () => <div data-testid="section-app" />,
}));
vi.mock('./app/WorkspaceSettingsSection', () => ({
  WorkspaceSettingsSection: () => <div data-testid="section-workspace" />,
}));
vi.mock('./config/ConfigSettings', () => ({
  default: () => <div data-testid="section-config" />,
}));

vi.mock('../ConfigContext', () => ({
  useConfig: () => ({
    read: vi.fn(async () => undefined),
    upsert: vi.fn(async () => undefined),
  }),
}));

const renderSettings = (viewOptions = {}) =>
  render(<SettingsView onClose={() => {}} setView={() => {}} viewOptions={viewOptions} />);

/** Document order of the sections that actually rendered. */
function sectionOrder(): string[] {
  return [...document.querySelectorAll('[data-testid^="section-"], [data-privacy-panel]')].map(
    (el) => el.getAttribute('data-testid') ?? 'section-privacy'
  );
}

describe('SettingsView', () => {
  /**
   * ⚠ **Privacy is a SECTION of App, not a tab.** It shipped as a fourth tab,
   * which made it read as a separate product rather than a property of this
   * install. There is no `settings-privacy-tab` any more, and a test that
   * clicked one would fail loudly rather than quietly asserting nothing.
   */
  it('mounts the Privacy panel inside the App tab, with no tab of its own', async () => {
    const user = userEvent.setup();
    renderSettings();
    expect(screen.queryByTestId('settings-privacy-tab')).toBeNull();

    await user.click(screen.getByTestId('settings-app-tab'));
    expect(await screen.findByRole('switch', { name: /Privacy tiers/ })).toBeInTheDocument();
  });

  /**
   * The operator's order, and it is the point of the change: Configuration,
   * Privacy, Workspace, then everything `AppSettingsSection` owns — which ends
   * with Updates, so Updates stays at the bottom of the page.
   *
   * Asserted as document order rather than by eyeballing the JSX, because the
   * JSX is exactly what a later edit reorders.
   */
  it('orders the App tab: Configuration, Privacy, Workspace, then the rest', async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(screen.getByTestId('settings-app-tab'));
    await screen.findByRole('switch', { name: /Privacy tiers/ });

    const order = sectionOrder();
    const expected = CONFIGURATION_ENABLED
      ? ['section-config', 'section-privacy', 'section-workspace', 'section-app']
      : ['section-privacy', 'section-workspace', 'section-app'];
    expect(order).toEqual(expected);
  });

  /**
   * ⚠ A deep link to `section: 'privacy'` predates the move and must still land
   * somewhere it exists. Selecting a tab value that no longer has a trigger
   * leaves the whole panel blank, which is the shape of this bug.
   */
  it('still honours an old deep link to the privacy section', async () => {
    renderSettings({ section: 'privacy' });
    expect(await screen.findByRole('switch', { name: /Privacy tiers/ })).toBeInTheDocument();
  });
});

/**
 * Settings reads the CHAT measure (760px), not the fluid page measure
 * (operator decision, 2026-09-07): it is a column of labelled rows, so width
 * beyond the measure separates each control from the label it names instead of
 * showing more.
 *
 * ⚠ **What this can and cannot see.** jsdom has no layout engine and never runs
 * Tailwind, so `getBoundingClientRect()` here is zero for every box and the
 * WIDTH is unassertable — the alignment was measured in a real browser instead
 * (see the PR). What is assertable is the attribute that selects the width, and
 * that all three boxes agree on it: the header, the tab strip and the scrolling
 * body are three separate `ReadableContent`s meeting at one left edge, so a
 * size on one and not its siblings is a visible step in that edge rather than
 * an invisible inconsistency. The count is pinned so that a fourth column added
 * on the default size fails here rather than shipping a step, and
 * `styles/measures.test.ts` closes the remaining case this file cannot see: a
 * `<ReadableContent` written with no `size` prop at all.
 */
describe('SettingsView sits on the chat measure', () => {
  const readableColumns = () =>
    [...document.querySelectorAll('.biorouter-readable-content')] as HTMLElement[];

  it('renders exactly three reading columns, all of them the chat size', () => {
    renderSettings();

    const columns = readableColumns();
    expect(columns).toHaveLength(3);
    for (const column of columns) expect(column.dataset.size).toBe('chat');
  });

  /**
   * The tab strip and the body are inside `Tabs`, so switching tabs re-renders
   * the body. Asserted on the App tab as well because that is the tab with the
   * most sections under it, and the one a later edit is most likely to wrap in
   * a column of its own.
   */
  it('keeps every column on the chat size after switching tabs', async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(screen.getByTestId('settings-app-tab'));
    await screen.findByRole('switch', { name: /Privacy tiers/ });

    const columns = readableColumns();
    expect(columns).toHaveLength(3);
    for (const column of columns) expect(column.dataset.size).toBe('chat');
  });
});
