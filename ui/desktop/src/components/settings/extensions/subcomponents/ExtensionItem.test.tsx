import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import type { FixedExtensionEntry } from '../../../ConfigContext';
import ExtensionItem from './ExtensionItem';
import { EXTENSION_ROW_COPY as COPY } from '../copy';
import { loadRegistry } from '../../../baam/registry';

const builtin = {
  type: 'builtin',
  name: 'example',
  display_name: 'Example',
  description: 'Example extension',
  enabled: true,
} as FixedExtensionEntry;

const external = {
  type: 'stdio',
  name: 'spokeagent',
  description: 'SPOKE graph',
  cmd: 'uv',
  args: ['run', '--directory', '/home/me/.config/biorouter/extensions/spokeagent', 'main.py'],
  enabled: true,
} as FixedExtensionEntry;

describe('ExtensionItem', () => {
  it('shows an optimistic disabled state while the toggle request is pending', () => {
    const pending = new Promise<boolean>(() => undefined);

    render(<ExtensionItem extension={builtin} onToggle={vi.fn(() => pending)} />);
    const toggle = screen.getByRole('switch', { name: 'Example' });

    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(toggle).toBeDisabled();
  });

  it('names its switch by the row title, whatever the state', () => {
    // Spec 2.6: a switch is named by the visible label, never "Toggle …" or "Enable …", and the
    // name does not change when the state does (the state is aria-checked).
    const { rerender } = render(<ExtensionItem extension={external} onToggle={vi.fn()} />);
    const on = screen.getByRole('switch', { name: 'Spokeagent' });
    expect(on).toHaveAttribute('aria-checked', 'true');

    rerender(<ExtensionItem extension={{ ...external, enabled: false }} onToggle={vi.fn()} />);
    const off = screen.getByRole('switch', { name: 'Spokeagent' });
    expect(off).toHaveAttribute('aria-checked', 'false');
    expect(screen.queryByRole('switch', { name: /toggle|enable/i })).toBeNull();
  });

  it('is a title line and one muted line: no command, no source path', () => {
    const { container } = render(<ExtensionItem extension={external} onToggle={vi.fn()} />);

    // The command line moved into the Configure dialog, where it is edited.
    expect(screen.queryByText(/uv run/)).toBeNull();
    expect(container.textContent).not.toContain('/home/me');
    const secondary = container.querySelectorAll('.br-ext-secondary');
    expect(secondary).toHaveLength(1);
    expect(secondary[0]).toHaveTextContent('SPOKE graph');
  });

  it('keeps the description and the provenance sentence on the one muted line', async () => {
    // No `window.electron`, so this is the snapshot bundled with the app: the provenance a person
    // reads must not depend on the network being up.
    const catalog = await loadRegistry();
    const { container } = render(
      <ExtensionItem extension={external} onToggle={vi.fn()} catalog={catalog} />
    );

    const secondary = container.querySelector('.br-ext-secondary') as HTMLElement;
    expect(within(secondary).getByText('SPOKE graph')).toHaveClass('br-ext-description');
    // §13.5, verbatim, visible: a privacy disclosure is text, not help.
    expect(
      within(secondary).getByText('Public: published on the Biorouter marketplace')
    ).toHaveClass('br-ext-provenance');
    expect(container.querySelectorAll('.br-ext-secondary')).toHaveLength(1);
  });

  it('draws the Puzzle for an external extension and its own glyph for a built-in', () => {
    const { container, rerender } = render(
      <ExtensionItem extension={external} onToggle={vi.fn()} />
    );
    expect(container.querySelector('svg.br-ext-glyph.lucide-puzzle')).not.toBeNull();

    rerender(
      <ExtensionItem
        extension={
          { ...builtin, name: 'knowledge', display_name: 'Knowledge' } as FixedExtensionEntry
        }
        onToggle={vi.fn()}
      />
    );
    expect(container.querySelector('svg.br-ext-glyph.br-icon-knowledge')).not.toBeNull();

    // An external server a user happened to name like a built-in is still a Puzzle.
    rerender(
      <ExtensionItem
        extension={{ ...external, name: 'knowledge' } as FixedExtensionEntry}
        onToggle={vi.fn()}
      />
    );
    expect(container.querySelector('svg.br-ext-glyph.lucide-puzzle')).not.toBeNull();
  });

  it('puts the built-in provenance sentence in the privacy badge help, not on the row', () => {
    const { container } = render(<ExtensionItem extension={builtin} onToggle={vi.fn()} />);

    // Not a visible line…
    expect(container.querySelector('.br-ext-provenance')).toBeNull();
    // …but still in the page, and the badge's trigger is described by it.
    expect(screen.getByText(/built into Biorouter/i)).toHaveClass('sr-only');
    const trigger = container.querySelector('.br-ext-badge-target') as HTMLElement;
    expect(trigger).toHaveAccessibleDescription(COPY.builtInProvenance);
    expect(trigger).toHaveAttribute('tabindex', '0');
  });

  it('states a refused pairing as a visible warning badge with the full sentence as its help', () => {
    render(
      <ExtensionItem
        extension={{ ...external, name: 'ucsfomopagent' } as FixedExtensionEntry}
        onToggle={vi.fn()}
        defaultProvider={{ name: 'Anthropic', tier: 'public' }}
      />
    );

    const badge = screen.getByText(COPY.unavailable);
    expect(badge).toBeVisible();
    expect(badge).toHaveAccessibleDescription(COPY.unavailableHelp(true, 'Anthropic'));
    expect(badge).toHaveAccessibleDescription(/default model is public/);
  });

  it('opens Configure… and Remove… from its ⋯ menu, and only for an extension that can be removed', async () => {
    const user = userEvent.setup({ delay: null });
    const onConfigure = vi.fn();
    const onRemove = vi.fn();
    render(
      <ExtensionItem
        extension={external}
        onToggle={vi.fn()}
        onConfigure={onConfigure}
        onRemove={onRemove}
      />
    );

    await user.click(screen.getByRole('button', { name: 'More actions' }));
    await user.click(await screen.findByRole('menuitem', { name: COPY.configure }));
    expect(onConfigure).toHaveBeenCalledWith(external);

    await user.click(screen.getByRole('button', { name: 'More actions' }));
    await user.click(await screen.findByRole('menuitem', { name: COPY.remove }));
    expect(onRemove).toHaveBeenCalledWith(external);
  });

  it('offers a built-in only Configure…, and a bundled one no menu at all', async () => {
    const user = userEvent.setup({ delay: null });
    const { unmount } = render(
      <ExtensionItem
        extension={builtin}
        onToggle={vi.fn()}
        onConfigure={vi.fn()}
        onRemove={vi.fn()}
      />
    );
    await user.click(screen.getByRole('button', { name: 'More actions' }));
    expect(await screen.findByRole('menuitem', { name: COPY.configure })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: COPY.remove })).toBeNull();
    unmount();

    render(
      <ExtensionItem
        extension={{ ...external, bundled: true } as FixedExtensionEntry}
        onToggle={vi.fn()}
        onConfigure={vi.fn()}
        onRemove={vi.fn()}
      />
    );
    expect(screen.queryByRole('button', { name: 'More actions' })).toBeNull();
  });
});
