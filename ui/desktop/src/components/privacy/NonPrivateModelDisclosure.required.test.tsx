/**
 * The non-private model disclosure is answered, never dismissed (issue #56, DR-17 requirement 3),
 * now that it renders through `ModalShell` (ruling 23).
 *
 * The property moved from a prop (`dismissible={false}` on a raw `DialogContent`) to a derived one
 * (`purpose="required"`, from which the shell derives `dismissible`), so these cases fail the day
 * the purpose stops being `required`, each way out tried on its own.
 *
 * ⚠ Why the probe. The disclosure's `open` is the gate's, and the dialog has no close handler of
 * its own, so a dismissible version would STILL be on screen after Escape: "the dialog is still
 * visible" passes either way (the older case in `NonPrivateModelDisclosure.test.tsx` caught only the
 * missing ×). What changes is whether Radix asks to close. The real `Dialog` is wrapped so every
 * `onOpenChange` it raises is recorded; the shell, the primitive and Radix all run as shipped. The
 * first case checks the probe on a shell with the default purpose, where each way out does ask.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const probe = vi.hoisted(() => ({ openChanges: [] as boolean[] }));

vi.mock('../ui/dialog', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../ui/dialog')>();
  const React = await import('react');
  function Dialog({ onOpenChange, ...props }: React.ComponentProps<typeof actual.Dialog>) {
    return React.createElement(actual.Dialog, {
      ...props,
      onOpenChange: (next: boolean) => {
        probe.openChanges.push(next);
        onOpenChange?.(next);
      },
    });
  }
  return { ...actual, Dialog };
});

import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { NonPrivateModelDisclosure } from './NonPrivateModelDisclosure';

/** A fixture, never the product's words (see `NonPrivateModelDisclosure.test.tsx`). */
const COPY = {
  titleTemplate: '{provider} is not hosted by your institution.',
  long: 'SERVED-COPY-MARKER first paragraph.\n\nIt **does not** stop the second one.',
  short: 'SERVED-SHORT-MARKER',
};

function renderDisclosure(onAcknowledge = vi.fn()) {
  render(
    <NonPrivateModelDisclosure
      open
      providerDisplayName="OpenAI"
      copy={COPY}
      onAcknowledge={onAcknowledge}
    />
  );
  return { onAcknowledge, dialog: screen.getByRole('dialog') };
}

/** Radix registers its outside-press listener a tick after the content mounts. */
const outsideListenerArmed = () => new Promise((resolve) => setTimeout(resolve, 0));

const closeRequests = () => probe.openChanges.filter((next) => next === false);

beforeEach(() => {
  probe.openChanges.length = 0;
});

afterEach(cleanup);

describe('the probe sees a dismissal (instrument)', () => {
  function renderDismissible() {
    render(
      <ModalShell
        open
        onOpenChange={() => {}}
        title="A dismissible dialog"
        footer={<Button type="button">Go</Button>}
      >
        <p>Body</p>
      </ModalShell>
    );
  }

  it('records Escape, a backdrop press and the × on a shell with the default purpose', async () => {
    const user = userEvent.setup();
    renderDismissible();

    await user.keyboard('{Escape}');
    expect(closeRequests()).toHaveLength(1);

    await outsideListenerArmed();
    fireEvent.pointerDown(document.body);
    expect(closeRequests()).toHaveLength(2);

    await user.click(screen.getByRole('button', { name: 'Close' }));
    expect(closeRequests()).toHaveLength(3);
  });
});

describe('the disclosure cannot be dismissed, only answered (ruling 23)', () => {
  it('Escape does not ask to close it', async () => {
    const user = userEvent.setup();
    const { onAcknowledge } = renderDisclosure();

    await user.keyboard('{Escape}');
    // Escape from the button too, not only from the prose that holds focus at open.
    screen.getByRole('button', { name: 'I understand' }).focus();
    await user.keyboard('{Escape}');

    expect(probe.openChanges).toEqual([]);
    expect(screen.getByRole('dialog')).toBeVisible();
    expect(onAcknowledge).not.toHaveBeenCalled();
  });

  it('a press on the backdrop does not ask to close it', async () => {
    const { onAcknowledge } = renderDisclosure();
    await outsideListenerArmed();

    // A modal dialog turns pointer events off on the page, so a real click cannot reach it in
    // jsdom; Radix listens for the press itself. The overlay is the backdrop a person would hit.
    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);
    fireEvent.pointerDown(document.body);

    expect(probe.openChanges).toEqual([]);
    expect(screen.getByRole('dialog')).toBeVisible();
    expect(onAcknowledge).not.toHaveBeenCalled();
  });

  it('draws no ×: its one control is the acknowledgement', () => {
    const { dialog } = renderDisclosure();
    expect(within(dialog).queryByRole('button', { name: 'Close' })).toBeNull();
    expect(
      within(dialog)
        .getAllByRole('button')
        .map((button) => button.textContent)
    ).toEqual(['I understand']);
    // The title column keeps no gutter for a × that is not there.
    expect(within(dialog).getByRole('heading').parentElement).not.toHaveClass('pr-12');
  });

  it('opens with focus on the prose, not on the button', async () => {
    const { dialog } = renderDisclosure();
    await waitFor(() => expect(dialog).toContainElement(document.activeElement as HTMLElement));

    const focused = document.activeElement as HTMLElement;
    expect(focused).toHaveTextContent('SERVED-COPY-MARKER');
    expect(focused).toHaveAttribute('tabindex', '-1');
    expect(focused).not.toBe(screen.getByRole('button', { name: 'I understand' }));
    expect(focused).not.toBe(dialog);
  });

  it('is the shared dialog: top anchored, size lg, its test id on the dialog node', () => {
    const { dialog } = renderDisclosure();
    expect(dialog).toHaveAttribute('data-anchor', 'top');
    expect(dialog).toHaveAttribute('data-size', 'lg');
    expect(screen.getByTestId('non-private-model-disclosure')).toBe(dialog);
    expect(dialog).toHaveAccessibleName('OpenAI is not hosted by your institution.');
    // No description is claimed: the prose is read from focus, as before.
    expect(dialog).not.toHaveAttribute('aria-describedby');
  });

  it('is closed by the gate, through the button, and only that way', async () => {
    const user = userEvent.setup();
    const { onAcknowledge } = renderDisclosure();
    await user.click(screen.getByRole('button', { name: 'I understand' }));
    expect(onAcknowledge).toHaveBeenCalledTimes(1);
    expect(probe.openChanges).toEqual([]);
  });

  it('keeps a refused acknowledgement in the dialog, beside the relabelled button', () => {
    render(
      <NonPrivateModelDisclosure
        open
        providerDisplayName="OpenAI"
        copy={COPY}
        onAcknowledge={vi.fn()}
        acknowledgeError="Not saved: this app could not prove a person answered."
      />
    );
    const dialog = screen.getByRole('dialog');
    const failure = within(dialog).getByTestId('disclosure-ack-error');
    const button = within(dialog).getByRole('button', { name: 'Continue' });
    // Outside the scrolling body, on the button's row block, so it cannot scroll out of view.
    expect(failure.closest('.overflow-y-auto')).toBeNull();
    expect(failure.parentElement).toBe(button.parentElement);
    expect(failure.compareDocumentPosition(button) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});
