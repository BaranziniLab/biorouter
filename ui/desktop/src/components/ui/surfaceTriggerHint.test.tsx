import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { useSurfaceTriggerHint } from './surfaceTriggerHint';
import { AppTooltipProvider, Tooltip, TooltipContent, TooltipTrigger } from './Tooltip';

/**
 * The branches of `useSurfaceTriggerHint` a pointer cannot reach in Chromium: a surface that
 * opens while the hint shows without a press on the trigger (the app asking the Tools chip to
 * show this chat's extensions while the pointer rests on it, or Enter on a trigger Tab reached).
 * The pointer cases, and the keyboard opens on the real menus, are measured in
 * `surfaceTriggerHint.browser.test.tsx`.
 */
function Harness({ surfaceOpen }: { surfaceOpen: boolean }) {
  const hint = useSurfaceTriggerHint(surfaceOpen);
  return (
    <Tooltip {...hint.tooltip}>
      <TooltipTrigger asChild {...hint.trigger}>
        <button type="button">Tools</button>
      </TooltipTrigger>
      <TooltipContent>Tools hint</TooltipContent>
    </Tooltip>
  );
}

describe('useSurfaceTriggerHint', () => {
  it('hides a showing hint when the surface opens, and does not bring it back when it closes', async () => {
    const { rerender } = render(<Harness surfaceOpen={false} />);
    fireEvent.pointerMove(screen.getByRole('button', { name: 'Tools' }), { pointerType: 'mouse' });
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(
      'Tools hint'
    );

    rerender(<Harness surfaceOpen={true} />);
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());

    rerender(<Harness surfaceOpen={false} />);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  it('refuses the hover while the surface is open, so no open timer runs', async () => {
    render(<Harness surfaceOpen={true} />);
    fireEvent.pointerMove(screen.getByRole('button', { name: 'Tools' }), { pointerType: 'mouse' });
    await new Promise((resolve) => setTimeout(resolve, 700));
    expect(screen.queryByRole('tooltip')).toBeNull();
  });

  // A DropdownMenu trigger opens on Enter's keydown, and no click follows, so nothing on the
  // trigger closes a hint that Tab showed; the menu then takes focus and the trigger's blur asks
  // Radix to close it. A hint hidden first by its parent left Radix nothing to close, so the shared
  // provider never heard that a tooltip closed and the next label tooltip anywhere opened at once.
  it('closes a hint Tab showed through Radix, so the next tooltip keeps its 500ms delay', async () => {
    function Page({ surfaceOpen }: { surfaceOpen: boolean }) {
      return (
        <AppTooltipProvider>
          <button type="button">Before</button>
          <Harness surfaceOpen={surfaceOpen} />
          <button type="button">Menu</button>
          <Tooltip>
            <TooltipTrigger asChild>
              <button type="button">Plain</button>
            </TooltipTrigger>
            <TooltipContent>Plain hint</TooltipContent>
          </Tooltip>
        </AppTooltipProvider>
      );
    }
    const user = userEvent.setup();
    const { rerender } = render(<Page surfaceOpen={false} />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByRole('button', { name: 'Tools' })).toHaveFocus();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Tools hint');

    // The menu opens on the key, then takes focus.
    rerender(<Page surfaceOpen={true} />);
    act(() => screen.getByRole('button', { name: 'Menu' }).focus());
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
    // Closed by a press elsewhere, which hands focus to nobody.
    rerender(<Page surfaceOpen={false} />);
    act(() => (document.activeElement as HTMLElement | null)?.blur());

    // Past the provider's 300ms skip window, a hover on another tooltip waits its 500ms.
    await new Promise((resolve) => setTimeout(resolve, 600));
    fireEvent.pointerMove(screen.getByRole('button', { name: 'Plain' }), { pointerType: 'mouse' });
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(screen.queryByRole('tooltip'), 'no tooltip before the delay').toBeNull();
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(
      'Plain hint'
    );
  });

  // Escape on a menu hands focus back to its trigger by script. `Tooltip`'s Tab gate declines the
  // open that focus asks for, but inside `onOpenChange`, after Radix has told the shared provider
  // a tooltip opened and with no close to follow: the next label tooltip anywhere opened at once
  // (12ms measured in the app, against 512ms after a press outside).
  it('refuses a script focus at the trigger, so the next tooltip keeps its 500ms delay', async () => {
    function Page() {
      return (
        <AppTooltipProvider>
          <Harness surfaceOpen={false} />
          <Tooltip>
            <TooltipTrigger asChild>
              <button type="button">Plain</button>
            </TooltipTrigger>
            <TooltipContent>Plain hint</TooltipContent>
          </Tooltip>
        </AppTooltipProvider>
      );
    }
    render(<Page />);
    // A menu closing with Escape: focus put back on the trigger by script, not by Tab.
    act(() => screen.getByRole('button', { name: 'Tools' }).focus());
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole('tooltip'), 'a script focus opens nothing').toBeNull();

    // Past the provider's 300ms skip window, a hover on another tooltip waits its 500ms.
    await new Promise((resolve) => setTimeout(resolve, 600));
    fireEvent.pointerMove(screen.getByRole('button', { name: 'Plain' }), { pointerType: 'mouse' });
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(screen.queryByRole('tooltip'), 'no tooltip before the delay').toBeNull();
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(
      'Plain hint'
    );
  });

  it('still shows the hint for a Tab onto the trigger, and not while the surface is open', async () => {
    function Page({ surfaceOpen }: { surfaceOpen: boolean }) {
      return (
        <AppTooltipProvider>
          <button type="button">Before</button>
          <Harness surfaceOpen={surfaceOpen} />
        </AppTooltipProvider>
      );
    }
    const user = userEvent.setup();
    const { rerender } = render(<Page surfaceOpen={true} />);
    act(() => screen.getByRole('button', { name: 'Before' }).focus());
    await user.tab();
    expect(screen.getByRole('button', { name: 'Tools' })).toHaveFocus();
    await new Promise((resolve) => setTimeout(resolve, 100));
    expect(screen.queryByRole('tooltip'), 'no hint over the open surface').toBeNull();

    rerender(<Page surfaceOpen={false} />);
    await user.tab({ shift: true });
    await user.tab();
    expect(await screen.findByRole('tooltip')).toHaveTextContent('Tools hint');
  });
});
