import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { IconAction, RowActions, RowContextMenu, type RowActionItem } from './row-actions';
import { Play } from '../icons/app-icons';

function Row({ items, onRun }: { items: RowActionItem[]; onRun?: () => void }) {
  return (
    <RowContextMenu items={items}>
      <div className="biorouter-list-row" data-testid="row" tabIndex={0}>
        <span>Weekly digest</span>
        <RowActions
          primary={<IconAction icon={Play} label="Run" onSelect={onRun ?? (() => {})} />}
          menu={items}
          meta={<time>2h</time>}
        />
      </div>
    </RowContextMenu>
  );
}

describe('RowActions', () => {
  it('renders the hover-revealed cluster with a primary action and a named ⋯ button', () => {
    const onRename = vi.fn();
    render(<Row items={[{ label: 'Rename', onSelect: onRename }]} />);
    const cluster = document.querySelector('.br-row-actions');
    expect(cluster).not.toBeNull();
    expect(cluster).toHaveAttribute('data-state', 'closed');
    expect(screen.getByRole('button', { name: 'Run' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'More actions' })).toBeInTheDocument();
    expect(screen.getByText('2h').closest('.br-row-actions-meta')).not.toBeNull();
    expect(screen.getByTestId('row')).toHaveAttribute('data-row-actions-host');
  });

  it('opens the items from ⋯ and marks the cluster open', async () => {
    const user = userEvent.setup();
    const onRename = vi.fn();
    render(
      <Row
        items={[
          { label: 'Rename', onSelect: onRename },
          { kind: 'separator' },
          { label: 'Delete', onSelect: () => {}, destructive: true },
        ]}
      />
    );
    await user.click(screen.getByRole('button', { name: 'More actions' }));
    expect(document.querySelector('.br-row-actions')).toHaveAttribute('data-state', 'open');
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toHaveAttribute(
      'data-variant',
      'destructive'
    );
    await user.click(screen.getByRole('menuitem', { name: 'Rename' }));
    expect(onRename).toHaveBeenCalledTimes(1);
  });

  it('runs the primary action without the click reaching the row', async () => {
    const user = userEvent.setup();
    const onRun = vi.fn();
    const onRowClick = vi.fn();
    render(
      <div onClick={onRowClick}>
        <Row items={[{ label: 'Rename', onSelect: () => {} }]} onRun={onRun} />
      </div>
    );
    await user.click(screen.getByRole('button', { name: 'Run' }));
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onRowClick).not.toHaveBeenCalled();
  });

  it('opens the same items as a context menu on right-click', async () => {
    render(<Row items={[{ label: 'Rename', onSelect: () => {} }]} />);
    fireEvent.contextMenu(screen.getByTestId('row'));
    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
  });

  it('opens the context menu from Shift+F10 on the focused row', async () => {
    render(<Row items={[{ label: 'Rename', onSelect: () => {} }]} />);
    const row = screen.getByTestId('row');
    act(() => row.focus());
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true });
    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
  });

  it('opens it from the ContextMenu key too', async () => {
    render(<Row items={[{ label: 'Rename', onSelect: () => {} }]} />);
    const row = screen.getByTestId('row');
    act(() => row.focus());
    fireEvent.keyDown(row, { key: 'ContextMenu' });
    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
  });
});

describe('the reveal rules', () => {
  it('reveal on hover, focus-within and an open menu, and always under hover:none', async () => {
    const { readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
    const block = css.slice(
      css.indexOf('/* @ws WS-PRIMITIVES begin: primitives */'),
      css.indexOf('/* @ws WS-PRIMITIVES end: primitives */')
    );
    expect(block).toMatch(
      /\.br-row-actions \{[^}]*opacity: 0;[^}]*transition: opacity var\(--dur-fast-min\)/
    );
    expect(block).toMatch(
      /:hover,\s*:focus-within,\s*\[data-state='open'\]\s*\)\s*\.br-row-actions,/
    );
    expect(block).toMatch(
      /@media \(hover: none\) \{[\s\S]*?\.br-row-actions,[\s\S]*?opacity: 1 !important;/
    );
  });
});

describe('IconAction tooltip', () => {
  it('can keep the long name for screen readers and a short tooltip', async () => {
    const user = userEvent.setup();
    render(<IconAction icon={Play} label="Run Daily summary" tooltip="Run" onSelect={() => {}} />);
    const button = screen.getByRole('button', { name: 'Run Daily summary' });
    await user.hover(button);
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent('Run');
  });
});
