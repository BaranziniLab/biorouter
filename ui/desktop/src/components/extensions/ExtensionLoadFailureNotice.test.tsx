import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import fs from 'node:fs';
import path from 'node:path';
import { ExtensionLoadFailureNotice } from './ExtensionLoadFailureNotice';
import {
  recordExtensionLoadResults,
  resetExtensionLoadFailuresForTests,
  getExtensionLoadFailures,
} from '../../utils/extensionLoadFailures';

/**
 * Defect 1, second half. The recurring toast pointed at the Extensions page and
 * the page said "No extensions yet" — the destination denied that the thing
 * that had just failed existed. These cases pin the name, the reason and at
 * least one thing to do about it onto that page.
 */
describe('ExtensionLoadFailureNotice', () => {
  beforeEach(() => {
    resetExtensionLoadFailuresForTests();
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

    expect(screen.getByText('Cdwagent failed to load')).toBeInTheDocument();
    expect(screen.getByText(error)).toBeInTheDocument();
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

    expect(screen.queryByText('Cdwagent failed to load')).not.toBeInTheDocument();
    expect(getExtensionLoadFailures()).toHaveLength(0);
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
