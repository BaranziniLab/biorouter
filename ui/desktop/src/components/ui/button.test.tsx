import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Button } from './button';
import { Collapsible, CollapsibleTrigger } from './collapsible';
import { Dialog, DialogTrigger } from './dialog';
import { DropdownMenu, DropdownMenuTrigger } from './dropdown-menu';
import { Sheet, SheetTrigger } from './sheet';
import { Tooltip, TooltipProvider, TooltipTrigger } from './Tooltip';

/**
 * V4-24 (WS-SETTINGS-A-3). `main.css` draws the forced-colours edge, the link underline and the
 * copy field's footer on `[data-slot='button']`. Every `*Trigger asChild` that wraps a Button
 * hands its own `data-slot` to the Button through Radix's Slot, and a slot written before the
 * spread lost to it, so those buttons were buttons without the button's rules. A Button is always
 * a button: its slot is written last.
 */
describe('Button’s data-slot', () => {
  it('is "button" on a plain Button and on one that renders another element', () => {
    render(
      <>
        <Button>Plain</Button>
        <Button asChild>
          <a href="#x">Anchor</a>
        </Button>
      </>
    );
    expect(screen.getByRole('button', { name: 'Plain' })).toHaveAttribute('data-slot', 'button');
    expect(screen.getByRole('link', { name: 'Anchor' })).toHaveAttribute('data-slot', 'button');
  });

  it('stays "button" when a Radix trigger hands it its own slot', () => {
    render(
      <TooltipProvider>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button>Menu</Button>
          </DropdownMenuTrigger>
        </DropdownMenu>
        <Dialog>
          <DialogTrigger asChild>
            <Button>Dialog</Button>
          </DialogTrigger>
        </Dialog>
        <Sheet>
          <SheetTrigger asChild>
            <Button>Sheet</Button>
          </SheetTrigger>
        </Sheet>
        <Collapsible>
          <CollapsibleTrigger asChild>
            <Button>Collapsible</Button>
          </CollapsibleTrigger>
        </Collapsible>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button>Tooltip</Button>
          </TooltipTrigger>
        </Tooltip>
      </TooltipProvider>
    );
    for (const name of ['Menu', 'Dialog', 'Sheet', 'Collapsible', 'Tooltip']) {
      expect(screen.getByRole('button', { name }), name).toHaveAttribute('data-slot', 'button');
    }
    // The trigger still triggers: its behaviour came through the same Slot.
    expect(screen.getByRole('button', { name: 'Menu' })).toHaveAttribute('aria-haspopup', 'menu');
    expect(screen.getByRole('button', { name: 'Dialog' })).toHaveAttribute(
      'aria-haspopup',
      'dialog'
    );
  });

  it('leaves a trigger that wraps something other than a Button with the trigger’s own slot', () => {
    render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button type="button">Bare</button>
        </DropdownMenuTrigger>
      </DropdownMenu>
    );
    expect(screen.getByRole('button', { name: 'Bare' })).toHaveAttribute(
      'data-slot',
      'dropdown-menu-trigger'
    );
  });

  it('writes the slot after the spread (the source order is the mechanism)', () => {
    const source = readFileSync(join(__dirname, 'button.tsx'), 'utf8');
    const spread = source.indexOf('{...props}');
    const slot = source.indexOf('data-slot="button"', spread);
    expect(spread).toBeGreaterThan(-1);
    expect(slot, 'data-slot="button" comes after {...props} in the Comp element').toBeGreaterThan(
      spread
    );
    // And nowhere before it: a second, earlier write would only be confusing.
    expect(source.indexOf('data-slot="button"')).toBe(slot);
  });
});
