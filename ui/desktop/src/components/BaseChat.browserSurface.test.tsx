import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { InAppTerminalHeaderButton, runInTerminalWhereOffered } from './BaseChat';
import { BROWSER_SURFACE_MARKER } from '../utils/surface';

/**
 * The in-app terminal is an Electron capability (`docs/deployment/browser-access.md`):
 * a `biorouter serve` page has no pty, and the web shim answers every start with
 * "In-app terminal is only available in Electron." SD-8 rules that a control which
 * can never work on a surface says so before it is touched, so in a browser the
 * chat offers no way into a terminal at all (rq2-088, rq2-089): no header toggle,
 * and no Run on a shell code block.
 *
 * BaseChat cannot be mounted in jsdom (see `BaseChat.privacy.test.tsx`), so the
 * toggle is the exported component the header renders, and the transcript's Run
 * handoff is the exported gate the transcript is handed.
 */

const inBrowser = () => {
  document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
};

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('the in-app terminal on a served browser page', () => {
  it('the_in_app_terminal_button_is_absent_in_a_browser', () => {
    inBrowser();
    const { unmount } = render(<InAppTerminalHeaderButton open={false} onToggle={vi.fn()} />);
    expect(screen.queryByRole('button', { name: 'Open in-app terminal' })).toBeNull();
    expect(screen.queryByRole('button', { name: /terminal/i })).toBeNull();
    unmount();

    delete document.documentElement.dataset.biorouterSurface;
    render(<InAppTerminalHeaderButton open={false} onToggle={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Open in-app terminal' })).toBeInTheDocument();
  });

  it('names the open dock in the desktop app, so the toggle still closes it', () => {
    render(<InAppTerminalHeaderButton open onToggle={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Close in-app terminal' })).toBeInTheDocument();
  });

  it('hands the transcript no Run in a browser, and the chat’s own handler on the desktop', () => {
    const handler = vi.fn(() => true);
    inBrowser();
    expect(runInTerminalWhereOffered(handler)).toBeNull();
    delete document.documentElement.dataset.biorouterSurface;
    expect(runInTerminalWhereOffered(handler)).toBe(handler);
  });

  it('BaseChat hands the transcript its Run through that gate', () => {
    // vitest runs with `ui/desktop` as its root, the idiom BaseChat.privacy.test.tsx uses.
    const source = readFileSync(
      path.join(process.cwd(), 'src', 'components', 'BaseChat.tsx'),
      'utf8'
    );
    expect(source).toMatch(
      /const transcriptRunInTerminal = runInTerminalWhereOffered\(handleRunInTerminal\);/
    );
    expect(source).toMatch(/onRunInTerminal=\{transcriptRunInTerminal\}/);
    expect(source).not.toMatch(/onRunInTerminal=\{handleRunInTerminal\}/);
    expect(source).toMatch(/<InAppTerminalHeaderButton\b/);
  });
});
