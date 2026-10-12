import { describe, it, expect } from 'vitest';
import userEvent from '@testing-library/user-event';
import { render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { GroupedExtensionLoadingToast } from '../GroupedExtensionLoadingToast';

const renderWithRouter = (component: React.ReactElement) => {
  return render(<MemoryRouter>{component}</MemoryRouter>);
};

describe('GroupedExtensionLoadingToast', () => {
  it('renders loading state without a count', () => {
    const extensions = [
      { name: 'developer', status: 'loading' as const },
      { name: 'memory', status: 'loading' as const },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={false} />
    );

    expect(screen.getByText('Loading extensions…')).toBeInTheDocument();
    // No details toggle while nothing has failed.
    expect(screen.queryByRole('button', { name: 'View details' })).not.toBeInTheDocument();
  });

  it('renders success as "All extensions loaded" (no number, no details)', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'success' as const },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    expect(screen.getByText('All extensions loaded')).toBeInTheDocument();
    expect(screen.queryByText(/loaded \d+/i)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'View details' })).not.toBeInTheDocument();
  });

  it('renders partial failure as "N of M loaded" AND names which failed', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'error' as const, error: 'Failed to connect' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    // how much still works — leading with the survivors, not the casualties
    expect(screen.getByText('1 of 2 extensions loaded')).toBeInTheDocument();
    // which failed (friendly name)
    expect(screen.getByText('Failed: Memory')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'View details' })).toBeInTheDocument();
  });

  it('names multiple failed extensions', () => {
    const extensions = [
      { name: 'developer', status: 'error' as const, error: 'boom' },
      { name: 'memory', status: 'error' as const, error: 'boom' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    expect(screen.getByText('0 of 2 extensions loaded')).toBeInTheDocument();
    expect(screen.getByText('Failed: Developer, Memory')).toBeInTheDocument();
  });

  it('reports the partial ratio the user was promised — "3 of 4 extensions loaded"', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'success' as const },
      { name: 'computercontroller', status: 'success' as const },
      { name: 'memory-broken', status: 'error' as const, error: 'spawn ENOENT' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={4} isComplete={true} />
    );

    expect(screen.getByText('3 of 4 extensions loaded')).toBeInTheDocument();
    // A partial failure must never read as a clean success.
    expect(screen.queryByText('All extensions loaded')).not.toBeInTheDocument();
  });

  it('renders a single successful extension as "All extensions loaded"', () => {
    const extensions = [{ name: 'developer', status: 'success' as const }];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={1} isComplete={true} />
    );

    expect(screen.getByText('All extensions loaded')).toBeInTheDocument();
  });

  // V3-R2-6: the toast and its report name an extension as the catalogue does,
  // not as its config key title-cased ("Cdwagent", "Computercontroller").
  it('names extensions as the catalogue spells them, in the toast and the report', async () => {
    const user = userEvent.setup();
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'cdwagent', status: 'error' as const, error: 'spawn ENOENT' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast
        extensions={extensions}
        totalCount={2}
        isComplete={true}
        builtinFailures={['computercontroller']}
      />
    );

    expect(screen.getByText('Failed: CDWAgent · Built-in: Biorouter Copilot')).toBeInTheDocument();
    expect(screen.queryByText(/Cdwagent|Computercontroller/)).not.toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'View details' }));

    expect(screen.getByRole('dialog')).toHaveTextContent('CDWAgent');
    expect(screen.queryByText('Cdwagent')).not.toBeInTheDocument();
  });

  it('while still loading, already-failed extensions are named', () => {
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'loading' as const },
      { name: 'Square MCP Server', status: 'error' as const, error: 'Connection failed' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={3} isComplete={false} />
    );

    expect(screen.getByText('Loading extensions…')).toBeInTheDocument();
    expect(screen.getByText('Failed: Square MCP Server')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'View details' })).toBeInTheDocument();
  });

  // THE TIER BOUNDARY (Astryx §3.7). A list, a per-item action and a scrolling
  // panel are all disqualifying for a toast; the toast keeps a title, a message
  // and one quiet action, and the report is a dialog it links to. Before this,
  // the whole 400px panel lived inside the transient layer.
  it('keeps the per-extension detail out of the toast until the report is opened', async () => {
    const user = userEvent.setup();
    const extensions = [
      { name: 'developer', status: 'success' as const },
      { name: 'memory', status: 'error' as const, error: 'spawn ENOENT' },
    ];

    renderWithRouter(
      <GroupedExtensionLoadingToast extensions={extensions} totalCount={2} isComplete={true} />
    );

    // Nothing per-item on the toast: no error text, no per-row buttons.
    expect(screen.queryByText(/spawn ENOENT/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Copy error' })).not.toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    // At most two quiet actions, and this report needs only one.
    expect(screen.getAllByRole('button')).toHaveLength(1);

    await user.click(screen.getByRole('button', { name: 'View details' }));

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(screen.getByText(/spawn ENOENT/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Copy error' })).toBeInTheDocument();
  });
});
