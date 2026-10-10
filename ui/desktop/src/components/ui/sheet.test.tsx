import { useState } from 'react';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from './sheet';

/**
 * The description contract for drawers.
 *
 * Radix writes `aria-describedby` onto every `SheetContent` unconditionally,
 * pointing at the id a `SheetDescription` would claim. Render no description and
 * the attribute dangles at nothing: the accessibility tree carries a broken
 * reference, and Radix logs *"Missing `Description` or
 * `aria-describedby={undefined}` for {DialogContent}"* on every open.
 *
 * These tests install a local warning spy because one case deliberately emits
 * the Radix diagnostic. The rest of the suite leaves warnings visible.
 */
let warn: MockInstance;

const warned = () =>
  warn.mock.calls.some((call) => String(call[0]).includes('Missing `Description`'));

const surface = () => document.querySelector('[data-slot="sheet-content"]')!;

beforeEach(() => {
  warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined);
});
afterEach(() => {
  warn.mockRestore();
  cleanup();
});

describe('SheetContent — settling the description', () => {
  it('links a rendered SheetDescription, and says nothing', () => {
    render(
      <Sheet open>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>Sidebar</SheetTitle>
            <SheetDescription>Displays the mobile sidebar.</SheetDescription>
          </SheetHeader>
        </SheetContent>
      </Sheet>
    );

    const id = surface().getAttribute('aria-describedby');
    expect(id).toBeTruthy();
    expect(document.getElementById(id!)).toHaveTextContent('Displays the mobile sidebar.');
    expect(warned()).toBe(false);
  });

  it('drops the attribute entirely on the explicit opt-out', () => {
    render(
      <Sheet open>
        <SheetContent aria-describedby={undefined}>
          <SheetHeader>
            <SheetTitle>Change log</SheetTitle>
          </SheetHeader>
        </SheetContent>
      </Sheet>
    );

    // Not "points at an empty description" — absent. A dangling id is the defect.
    expect(surface().hasAttribute('aria-describedby')).toBe(false);
    expect(warned()).toBe(false);
  });

  // Without this case the two above would pass against a primitive that could
  // never warn, and the guard below would be policing nothing.
  it('does dangle, and does warn, when a call site settles neither', () => {
    render(
      <Sheet open>
        <SheetContent>
          <SheetHeader>
            <SheetTitle>Change log</SheetTitle>
          </SheetHeader>
        </SheetContent>
      </Sheet>
    );

    const id = surface().getAttribute('aria-describedby');
    expect(id).toBeTruthy();
    expect(document.getElementById(id!)).toBeNull();
    expect(warned()).toBe(true);
  });
});

/** WS-KNOWLEDGE-1: the close is named, so a pane header can align it without counting children. */
describe('SheetContent close', () => {
  it('names its close button with data-slot="sheet-close"', () => {
    render(
      <Sheet open>
        <SheetContent aria-describedby={undefined}>
          <SheetTitle>Changes</SheetTitle>
        </SheetContent>
      </Sheet>
    );
    expect(screen.getByRole('button', { name: 'Close' })).toHaveAttribute(
      'data-slot',
      'sheet-close'
    );
  });
});

describe('SheetContent focus return', () => {
  it('gives focus back to the control that opened a sheet that has no Trigger', async () => {
    const user = userEvent.setup();
    function FromState() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open drawer
          </button>
          <Sheet open={open} onOpenChange={setOpen}>
            <SheetContent aria-describedby={undefined}>
              <SheetTitle>Drawer</SheetTitle>
            </SheetContent>
          </Sheet>
        </>
      );
    }
    render(<FromState />);
    const opener = screen.getByRole('button', { name: 'Open drawer' });
    await user.click(opener);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });
});
