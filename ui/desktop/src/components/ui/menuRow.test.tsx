import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { Command, CommandGroup, CommandItem, CommandList } from './command';
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
} from './context-menu';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
  MENU_GROUP_LABEL_CLASS_NAME,
  MENU_ROW_CLASS_NAME,
  MENU_SURFACE_CLASS_NAME,
} from './dropdown-menu';
import { SELECT_OPTION_CLASS_NAME } from './Select';

/** Every token of the one row string is on `element` (a clickable row may swap the cursor). */
function expectMenuRow(element: Element) {
  for (const token of MENU_ROW_CLASS_NAME.split(/\s+/).filter((t) => !t.startsWith('cursor-'))) {
    expect(element.classList.contains(token), `${token} on ${element.outerHTML.slice(0, 80)}`).toBe(
      true
    );
  }
}

async function openDropdown(children: React.ReactNode) {
  const user = userEvent.setup();
  render(
    <DropdownMenu>
      <DropdownMenuTrigger>Open</DropdownMenuTrigger>
      <DropdownMenuContent>{children}</DropdownMenuContent>
    </DropdownMenu>
  );
  await user.click(screen.getByRole('button', { name: 'Open' }));
  return user;
}

describe('one menu row (spec 2.6)', () => {
  it('is 32px minimum, 12px inset, radius 8, 13/18, the overlay-hover highlight', () => {
    const tokens = MENU_ROW_CLASS_NAME.split(/\s+/);
    for (const token of [
      'min-h-control-md',
      'px-3',
      'rounded-element',
      'gap-2',
      'text-secondary',
      'focus:bg-overlay-hover',
      'data-[disabled]:opacity-50',
      'br-menu-row',
    ]) {
      expect(tokens).toContain(token);
    }
  });

  it('is the row of a dropdown item', async () => {
    await openDropdown(<DropdownMenuItem>Rename</DropdownMenuItem>);
    expectMenuRow(screen.getByRole('menuitem', { name: 'Rename' }));
  });

  it('is the row of a context-menu item', async () => {
    render(
      <ContextMenu>
        <ContextMenuTrigger>Row</ContextMenuTrigger>
        <ContextMenuContent>
          <ContextMenuItem>Rename</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
    );
    fireEvent.contextMenu(screen.getByText('Row'));
    expectMenuRow(await screen.findByRole('menuitem', { name: 'Rename' }));
  });

  it('is the row of a command item', () => {
    render(
      <Command label="Pick" query="" onQueryChange={() => {}}>
        <CommandList>
          <CommandGroup heading="Models">
            <CommandItem onSelect={() => {}}>gpt-5.6</CommandItem>
          </CommandGroup>
        </CommandList>
      </Command>
    );
    expectMenuRow(screen.getByRole('option', { name: 'gpt-5.6' }));
    expect(screen.getByText('Models').className).toBe(MENU_GROUP_LABEL_CLASS_NAME);
  });

  it('is the row of a select option', () => {
    for (const token of MENU_ROW_CLASS_NAME.split(/\s+/)) {
      expect(SELECT_OPTION_CLASS_NAME.split(/\s+/)).toContain(token);
    }
  });

  it('puts the check of a checkbox item at the trailing edge, with no left gutter', async () => {
    await openDropdown(
      <DropdownMenuCheckboxItem checked value="Folder">
        Group by
      </DropdownMenuCheckboxItem>
    );
    const item = screen.getByRole('menuitemcheckbox', { name: /Group by/ });
    expect(item.className).not.toMatch(/(^|\s)pl-8(\s|$)/);
    const indicator = item.querySelector('[data-slot="menu-item-indicator"]')!;
    expect(indicator).not.toBeNull();
    // The label comes first; the value, then the check, follow it.
    expect(item.firstChild?.textContent).toBe('Group by');
    expect(item.lastElementChild).toHaveClass('br-menu-row-trailing');
    expect(item.lastElementChild?.textContent).toContain('Folder');
    expect(indicator.querySelector('svg')).not.toBeNull();
  });

  it('puts the check of a radio item at the trailing edge', async () => {
    await openDropdown(
      <DropdownMenuRadioGroup value="date">
        <DropdownMenuRadioItem value="date">Date</DropdownMenuRadioItem>
        <DropdownMenuRadioItem value="folder">Folder</DropdownMenuRadioItem>
      </DropdownMenuRadioGroup>
    );
    const chosen = screen.getByRole('menuitemradio', { name: 'Date' });
    expect(chosen.className).not.toMatch(/(^|\s)pl-8(\s|$)/);
    expect(chosen.lastElementChild).toHaveClass('br-menu-row-trailing');
    expect(chosen.querySelector('[data-slot="menu-item-indicator"] svg')).not.toBeNull();
    const other = screen.getByRole('menuitemradio', { name: 'Folder' });
    expect(other.querySelector('[data-slot="menu-item-indicator"] svg')).toBeNull();
  });

  it('labels groups in sentence case at 12px, and shortcuts carry no tracking', async () => {
    await openDropdown(
      <>
        <DropdownMenuLabel>Sort by</DropdownMenuLabel>
        <DropdownMenuItem>
          Copy <DropdownMenuShortcut>⌘C</DropdownMenuShortcut>
        </DropdownMenuItem>
      </>
    );
    const label = screen.getByText('Sort by');
    expect(label.className).toContain('text-supporting');
    expect(label.className).not.toContain('text-caps');
    expect(screen.getByText('⌘C').className).not.toMatch(/tracking-/);
  });

  it('draws one surface with no row gap and the menu motion', async () => {
    expect(MENU_SURFACE_CLASS_NAME).not.toMatch(/space-y/);
    expect(MENU_SURFACE_CLASS_NAME).toMatch(/rounded-container/);
    expect(MENU_SURFACE_CLASS_NAME).toMatch(/br-menu-motion/);
    await openDropdown(<DropdownMenuItem>Rename</DropdownMenuItem>);
    expect(screen.getByRole('menu')).toHaveClass('br-menu-surface', 'br-menu-motion');
  });

  it('opens from its trigger: .97 scale, 4px toward the trigger, 175ms in, opacity-only 125ms out', () => {
    const css = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
    expect(css).toMatch(
      /\.br-menu-motion \{[^}]*--tw-ease: var\(--ease-out\);[^}]*transform-origin: var\(\s*--radix-dropdown-menu-content-transform-origin/
    );
    expect(css).toMatch(
      /\.br-menu-motion\[data-state='open'\] \{\s*--tw-duration: var\(--dur-fast-max\);\s*--tw-enter-scale: 0\.97;/
    );
    expect(css).toMatch(
      /\.br-menu-motion\[data-state='open'\]\[data-side='bottom'\] \{\s*--tw-enter-translate-y: -4px;/
    );
    expect(css).toMatch(
      /\.br-menu-motion\[data-state='closed'\] \{\s*--tw-duration: var\(--dur-fast\);\s*--tw-exit-scale: 1;/
    );
  });
});
