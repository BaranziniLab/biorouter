/**
 * Shared helpers for tests that drive the whole Crew route (ui-redesign-spec, "Test migration
 * plan", "Shared helpers"). Tests only — nothing in the app imports this file.
 *
 * The redesign moved a few controls off the resting screen into real menus (C5): Reconnect, Sign
 * in… and Connection settings… live in the workspace menu, Refresh channel in the channel menu, and
 * the provider and model selects became one model picker. These helpers open the menu the way a
 * person does — with pointer events, which Radix's `DropdownMenu` listens for and `fireEvent.click`
 * does not produce — so a migrated test still reaches the same control, and so the same wire call.
 */
import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, expect, vi } from 'vitest';
import { channelHeaderCopy } from '../channel/headerCopy';

/** A fresh user-event session per action: it binds to the document the test is rendering into. */
function user() {
  return userEvent.setup();
}

/**
 * Open the workspace menu (the switcher at the top of the Crew sidebar, whose name begins with the
 * workspace's name) and choose `item` once it is enabled — the wait replaces the old tests'
 * `toBeEnabled()` on the rail's button.
 */
export async function workspaceAction(item: string, workspace: RegExp = /^Fixture/) {
  await user().click(screen.getByRole('button', { name: workspace }));
  const entry = await screen.findByRole('menuitem', { name: item });
  await waitFor(() => expect(entry).not.toHaveAttribute('aria-disabled', 'true'));
  await user().click(entry);
}

/**
 * Open the channel menu (the channel's `<h1>`, "# general ▾", named "#general, channel menu") and
 * choose `item`.
 */
export async function channelAction(item: string, channel = 'general') {
  await user().click(screen.getByRole('button', { name: channelHeaderCopy.menuName(channel) }));
  const entry = await screen.findByRole('menuitem', { name: item });
  await waitFor(() => expect(entry).not.toHaveAttribute('aria-disabled', 'true'));
  await user().click(entry);
}

/**
 * Choose a model in Ask my agent's one picker (it replaced the "Configured provider" and "Model"
 * selects). The option's name begins with the model; the provider is its group.
 */
export async function chooseModel(model: string) {
  await user().click(await screen.findByRole('button', { name: /^Model/ }));
  const escaped = model.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await user().click(await screen.findByRole('option', { name: new RegExp(`^${escaped}`) }));
}

/**
 * jsdom has no `ResizeObserver`, and Radix measures with it (the privacy radio rows, popovers).
 * Call once at a test file's top level; it stubs a no-op observer for that file only.
 */
export function installResizeObserverStub(): void {
  class ResizeObserverStub {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  beforeAll(() => {
    vi.stubGlobal('ResizeObserver', ResizeObserverStub);
  });
  afterAll(() => {
    vi.unstubAllGlobals();
  });
}
