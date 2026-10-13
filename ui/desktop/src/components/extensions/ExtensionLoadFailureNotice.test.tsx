import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, render, screen, fireEvent, within } from '@testing-library/react';
import fs from 'node:fs';
import path from 'node:path';
import { ExtensionLoadFailureNotice } from './ExtensionLoadFailureNotice';
import { LOAD_FAILURE_COPY as COPY } from './copy';
import {
  recordExtensionLoadResults,
  resetExtensionLoadFailuresForTests,
  getExtensionLoadFailures,
  isExtensionLoadFailureNoticeShown,
} from '../../utils/extensionLoadFailures';
import { withdrawExtensionFailureToasts } from '../../utils/extensionErrorUtils';

vi.mock('../../utils/extensionErrorUtils', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/extensionErrorUtils')>()),
  withdrawExtensionFailureToasts: vi.fn(),
}));

/**
 * Defect 1, second half. The recurring toast pointed at the Extensions page and
 * the page said "No extensions yet" — the destination denied that the thing
 * that had just failed existed. These cases pin the name, the reason and at
 * least one thing to do about it onto that page.
 */
/** What a person reads on a button: its text without the label kept only to size it. */
function shownText(button: HTMLElement): string {
  const copy = button.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('[aria-hidden="true"]').forEach((el) => el.remove());
  return copy.textContent ?? '';
}

describe('ExtensionLoadFailureNotice', () => {
  beforeEach(() => {
    resetExtensionLoadFailuresForTests();
    vi.mocked(withdrawExtensionFailureToasts).mockClear();
  });

  // Final check (2026-10-12): the corner's sticky report of the same failure stood over the page,
  // over Copy error and, once the notice was dismissed, over the list's first switch.
  it('takes the place of the failure toast while the page is open', () => {
    const { unmount } = render(<ExtensionLoadFailureNotice />);
    expect(isExtensionLoadFailureNoticeShown()).toBe(true);
    expect(withdrawExtensionFailureToasts).not.toHaveBeenCalled();

    // A failure recorded while the page is open is shown here, and its toast taken down.
    act(() => {
      recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    });
    expect(screen.getByText(COPY.failedToLoad('CDWAgent'))).toBeInTheDocument();
    expect(withdrawExtensionFailureToasts).toHaveBeenCalled();

    unmount();
    expect(isExtensionLoadFailureNoticeShown()).toBe(false);
  });

  it('withdraws the failure toast when the page opens on a standing failure', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    render(<ExtensionLoadFailureNotice />);
    expect(withdrawExtensionFailureToasts).toHaveBeenCalled();
  });

  it('renders nothing when nothing has failed', () => {
    const { container } = render(<ExtensionLoadFailureNotice />);
    expect(container).toBeEmptyDOMElement();
  });

  it('names the failed extension and shows the FULL error, not a truncation', () => {
    const error =
      'failed to start the MCP server: spawn uvx ENOENT — the command is not on PATH for this login shell';
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error }]);

    render(<ExtensionLoadFailureNotice />);

    expect(screen.getByText(COPY.failedToLoad('CDWAgent'))).toBeInTheDocument();
    expect(screen.getByText(error)).toBeInTheDocument();
  });

  // Principle 10. The daemon reports a config KEY, and title-casing it read "Cdwagent failed to
  // load" and "Computercontroller failed to load" on the page that otherwise says CDWAgent and
  // Biorouter Copilot. The notice asks the one resolver for the name a person already knows.
  it.each([
    ['cdwagent', 'CDWAgent'],
    ['computercontroller', 'Biorouter Copilot'],
    ['codegraphagent', 'CodeGraph Agent'],
    ['webdocuments', 'Web & Documents'],
    ['chatrecall', 'Chat Recall'],
  ])('names %s as %s, never the key in capitals', (key, shown) => {
    recordExtensionLoadResults([{ name: key, success: false, error: 'spawn ENOENT' }]);

    render(<ExtensionLoadFailureNotice />);

    expect(screen.getByText(COPY.failedToLoad(shown))).toBeInTheDocument();
  });

  it('reads an unlisted key as words, the one place the key is still capitalised', () => {
    recordExtensionLoadResults([{ name: 'my-lab_tool', success: false, error: 'spawn ENOENT' }]);

    render(<ExtensionLoadFailureNotice />);

    expect(screen.getByText(COPY.failedToLoad('My Lab Tool'))).toBeInTheDocument();
  });

  it('offers something actionable, not just the bad news', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    const onAskBiorouter = vi.fn();

    render(<ExtensionLoadFailureNotice onAskBiorouter={onAskBiorouter} />);

    fireEvent.click(screen.getByRole('button', { name: 'Ask Biorouter' }));
    expect(onAskBiorouter).toHaveBeenCalledTimes(1);
    expect(onAskBiorouter.mock.calls[0][0]).toContain('spawn ENOENT');

    expect(screen.getByRole('button', { name: 'Copy error' })).toBeInTheDocument();
  });

  it('dismissing drops the record rather than hiding it for one page load', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    render(<ExtensionLoadFailureNotice />);

    fireEvent.click(screen.getByRole('button', { name: 'Dismiss' }));

    expect(screen.queryByText('CDWAgent failed to load')).not.toBeInTheDocument();
    expect(getExtensionLoadFailures()).toHaveLength(0);
  });

  /**
   * Compact (spec 3.11): ONE danger note holds the title, the error and every action. The two
   * text actions and the round X share one group at the trailing edge, on the title's row, with
   * no second row of buttons under the error, and the note is followed by 16px, not 24. The
   * group sits in the notice's own grid rather than in the Note's action slot so that it can
   * step aside from a toast standing over the column's right end, as far as the title before it
   * allows (final smoke; `data-toast-aside`, `alerts/toastAside.browser.test.tsx` measures it).
   */
  it('keeps its actions in one group after the title: two quiet text actions and a round X', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    const { container } = render(<ExtensionLoadFailureNotice onAskBiorouter={vi.fn()} />);

    const alert = screen.getByRole('alert');
    const buttons = within(alert).getAllByRole('button');
    expect(buttons.map((b) => b.getAttribute('aria-label') ?? shownText(b))).toEqual([
      COPY.askBiorouter,
      COPY.copyError,
      COPY.dismiss,
    ]);
    // All three sit in the one group that steps aside from a toast, right after the title (the
    // element it stops short of); none is under the error.
    const group = alert.querySelector('[data-toast-aside]') as HTMLElement;
    for (const button of buttons) expect(group).toContainElement(button);
    expect(group.previousElementSibling).toHaveTextContent(COPY.failedToLoad('CDWAgent'));
    expect(group.nextElementSibling).toHaveTextContent('spawn ENOENT');
    expect(group.nextElementSibling?.querySelector('button')).toBeNull();
    // Quiet: ghost, not the `secondary` wash the second row used to carry.
    expect(buttons[0].className).toMatch(/bg-transparent/);
    expect(buttons[1].className).toMatch(/bg-transparent/);
    expect(container.firstElementChild).toHaveClass('mb-4');
  });

  it('says "Copied" with no exclamation mark', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
    });
    render(<ExtensionLoadFailureNotice />);

    fireEvent.click(screen.getByRole('button', { name: COPY.copyError }));

    expect(screen.getByRole('button', { name: 'Copied' })).toBeInTheDocument();
    expect(screen.queryByText(/!/)).toBeNull();
  });

  // V3-R4-5. "Copy error" turned into the narrower "Copied" and Ask Biorouter, its neighbour,
  // slid 22px for two seconds. Both labels now share one grid cell, so the button keeps the width
  // of the longer one. jsdom has no layout: this pins the structure (the label that is not on
  // show is still in the cell, hidden from a screen reader, never read as part of the name) and
  // extensionNoticeGeometry.browser.test measures the width in Chromium.
  it('keeps the other label in the button so its width does not change, unread by a screen reader', () => {
    recordExtensionLoadResults([{ name: 'cdwagent', success: false, error: 'spawn ENOENT' }]);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
    });
    render(<ExtensionLoadFailureNotice />);

    const button = screen.getByRole('button', { name: COPY.copyError });
    const ghost = button.querySelector('[aria-hidden="true"]');
    expect(ghost?.textContent).toBe(COPY.copied);
    expect(button.querySelector('.br-ext-notice-swap')).not.toBeNull();

    fireEvent.click(button);

    // Same button, the labels traded places: the name is "Copied" alone, not both words.
    const after = screen.getByRole('button', { name: COPY.copied });
    expect(after).toBe(button);
    expect(after.querySelector('[aria-hidden="true"]')?.textContent).toBe(COPY.copyError);
  });

  /**
   * The wiring, asserted at the source. Rendering `ExtensionsView` here would
   * drag in `ConfigContext`, the marketplace registry and three modals for a
   * claim about one JSX line, and a mock deep enough to make that work is a
   * mock that can keep passing after the line is deleted.
   */
  it('is mounted by the Extensions page — the surface the failure is about', () => {
    const source = fs.readFileSync(path.join(__dirname, 'ExtensionsView.tsx'), 'utf8');
    expect(source).toContain('<ExtensionLoadFailureNotice');
  });
});
