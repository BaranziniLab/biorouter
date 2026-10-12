import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { useState } from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from './dialog';

afterEach(cleanup);

function ControlledDialog({ dismissible = true, onClose = () => {} }) {
  const [open, setOpen] = useState(false);

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        setOpen(nextOpen);
        if (!nextOpen) onClose();
      }}
    >
      <DialogTrigger>Open dialog</DialogTrigger>
      <DialogContent dismissible={dismissible} aria-describedby={undefined}>
        <DialogTitle>Test dialog</DialogTitle>
        <button type="button">Inside action</button>
      </DialogContent>
    </Dialog>
  );
}

/**
 * Radix returns focus to a dialog's own `Trigger` and to nothing else. A dialog opened from state
 * (a button's `onClick`, a keyboard shortcut) left focus on `<body>`, and a keyboard user began
 * again at the top of the page. `DialogContent` hands focus back to the element that had it when
 * the dialog opened (the same rule Settings applied dialog by dialog).
 */
function StateDialog({
  onCloseAutoFocus,
  removeOpenerOnOpen = false,
}: {
  onCloseAutoFocus?: (event: Event) => void;
  removeOpenerOnOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [showOpener, setShowOpener] = useState(true);
  return (
    <>
      {showOpener ? (
        <button
          type="button"
          onClick={() => {
            setOpen(true);
            if (removeOpenerOnOpen) setShowOpener(false);
          }}
        >
          Open from state
        </button>
      ) : null}
      <button type="button">Elsewhere</button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent aria-describedby={undefined} onCloseAutoFocus={onCloseAutoFocus}>
          <DialogTitle>State dialog</DialogTitle>
          <button type="button">Inside action</button>
        </DialogContent>
      </Dialog>
    </>
  );
}

describe('DialogContent focus return', () => {
  it('gives focus back to the control that opened a dialog that has no Trigger', async () => {
    const user = userEvent.setup();
    render(<StateDialog />);
    const opener = screen.getByRole('button', { name: 'Open from state' });
    await user.click(opener);
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(opener).toHaveFocus();
  });

  it('leaves focus to the caller when its own onCloseAutoFocus cancels the return', async () => {
    const user = userEvent.setup();
    const cancel = vi.fn((event: Event) => event.preventDefault());
    render(<StateDialog onCloseAutoFocus={cancel} />);
    const opener = screen.getByRole('button', { name: 'Open from state' });
    await user.click(opener);
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(opener).not.toHaveFocus();
  });

  it('leaves focus where the person put it when a non-modal dialog closes on a press elsewhere', async () => {
    const user = userEvent.setup();
    function NonModal() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>
            Open non-modal
          </button>
          <input aria-label="Elsewhere field" />
          <Dialog open={open} onOpenChange={setOpen} modal={false}>
            <DialogContent aria-describedby={undefined}>
              <DialogTitle>Non-modal</DialogTitle>
            </DialogContent>
          </Dialog>
        </>
      );
    }
    render(<NonModal />);
    await user.click(screen.getByRole('button', { name: 'Open non-modal' }));
    expect(screen.getByRole('dialog')).toBeInTheDocument();
    const field = screen.getByRole('textbox', { name: 'Elsewhere field' });
    await user.click(field);
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(field).toHaveFocus();
  });

  it('leaves an opener that is gone alone, and does not throw', async () => {
    const user = userEvent.setup();
    render(<StateDialog removeOpenerOnOpen />);
    await user.click(screen.getByRole('button', { name: 'Open from state' }));
    await user.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(screen.queryByRole('button', { name: 'Open from state' })).toBeNull();
  });
});

describe('DialogContent dismissal contract', () => {
  it('dismisses with Escape and restores focus to the trigger', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);

    const trigger = screen.getByRole('button', { name: 'Open dialog' });
    await user.click(trigger);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(trigger).toHaveFocus();
  });

  it('dismisses when the backdrop is pressed', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);

    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  /**
   * Defect D3b (2026-09-13). Toasts sit above every modal and are rendered
   * outside any dialog's tree, so a press on one reached Radix as a press
   * OUTSIDE the dialog: measured in the running app, one click on an error
   * toast's × closed "Make this chat public?" underneath while the toast stayed
   * on screen a minute later. The toast layer's own markup is reproduced here —
   * `section.Toastify` > `.Toastify__toast-container` > the card — because the
   * container is the hook the guard keys on.
   */
  it('does not treat a press on a toast as a press on the backdrop', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog onClose={onClose} />);
    await user.click(screen.getByRole('button', { name: 'Open dialog' }));

    const layer = document.createElement('section');
    layer.className = 'Toastify';
    layer.innerHTML =
      '<div class="Toastify__toast-container Toastify__toast-container--top-right">' +
      '<div role="alert"><button type="button" class="Toastify__close-button">×</button></div>' +
      '</div>';
    document.body.appendChild(layer);
    try {
      fireEvent.pointerDown(layer.querySelector('.Toastify__close-button')!);
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(screen.getByRole('dialog')).toBeInTheDocument();
      expect(onClose).not.toHaveBeenCalled();

      // …while the backdrop itself still dismisses, so the guard is not simply
      // "never dismiss on a press".
      fireEvent.pointerDown(document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]')!);
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
      expect(onClose).toHaveBeenCalledTimes(1);
    } finally {
      layer.remove();
    }
  });

  it('blocks Escape, backdrop, and the close button when not dismissible', async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<ControlledDialog dismissible={false} onClose={onClose} />);

    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();

    await user.keyboard('{Escape}');
    const overlay = document.querySelector<HTMLElement>('[data-slot="dialog-overlay"]');
    expect(overlay).not.toBeNull();
    fireEvent.pointerDown(overlay!);

    expect(screen.getByRole('dialog')).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('closes only the topmost dialog on Escape', async () => {
    const user = userEvent.setup();

    function NestedDialogs() {
      const [parentOpen, setParentOpen] = useState(true);
      const [childOpen, setChildOpen] = useState(true);

      return (
        <Dialog open={parentOpen} onOpenChange={setParentOpen}>
          <DialogContent aria-describedby={undefined}>
            <DialogTitle>Parent dialog</DialogTitle>
            <Dialog open={childOpen} onOpenChange={setChildOpen}>
              <DialogContent aria-describedby={undefined}>
                <DialogTitle>Child dialog</DialogTitle>
              </DialogContent>
            </Dialog>
          </DialogContent>
        </Dialog>
      );
    }

    render(<NestedDialogs />);
    expect(screen.getByText('Parent dialog')).toBeInTheDocument();
    expect(screen.getByText('Child dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');

    await waitFor(() => expect(screen.queryByText('Child dialog')).not.toBeInTheDocument());
    expect(screen.getByText('Parent dialog')).toBeInTheDocument();
  });
});

/**
 * Spec 2.6 / principle 9: the dialog opens with opacity and scale(.98) over `--dur-med-min` and
 * leaves over `--dur-fast`, the scrim fades over `--dur-med-min`, all `--ease-out`, and no
 * literal or retired `--motion-*` duration rides on the element. jsdom runs no animation, so
 * the recipe is pinned at the source.
 */
describe('dialog motion', () => {
  const CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');
  const body = (selector: string) => {
    const at = CSS.indexOf(`\n${selector} {`);
    expect(at, `no rule for ${selector}`).toBeGreaterThanOrEqual(0);
    return CSS.slice(CSS.indexOf('{', at) + 1, CSS.indexOf('}', at));
  };

  it('carries the motion class and no duration or zoom utilities', async () => {
    const user = userEvent.setup();
    render(<ControlledDialog />);
    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    const content = screen.getByRole('dialog');
    expect(content).toHaveClass('br-dialog-motion', 'data-[state=open]:animate-in');
    expect(content.className).not.toMatch(/duration-|zoom-(in|out)-|motion-(base|fast|slow)/);
    const overlay = document.querySelector('[data-slot="dialog-overlay"]');
    expect(overlay).toHaveClass('biorouter-modal-overlay');
  });

  it('authors the scale, durations and ease on the tokens', () => {
    expect(body(".br-dialog-motion[data-state='open']")).toMatch(
      /--tw-duration:\s*var\(--dur-med-min\);\s*--tw-enter-scale:\s*0\.98/
    );
    expect(body(".br-dialog-motion[data-state='closed']")).toMatch(/--tw-exit-scale:\s*0\.98/);
    expect(body(".biorouter-modal-overlay[data-state='open']")).toMatch(
      /--tw-duration:\s*var\(--dur-med-min\)/
    );
    expect(
      body(
        ".biorouter-modal-overlay[data-state='closed'],\n.br-dialog-motion[data-state='closed'],\n.br-sheet-motion[data-state='closed']"
      )
    ).toMatch(/--tw-duration:\s*var\(--dur-fast\)/);
    expect(body(".br-sheet-motion[data-state='open']")).toMatch(
      /--tw-duration:\s*var\(--dur-med\)/
    );
    expect(body('.biorouter-modal-overlay,\n.br-dialog-motion,\n.br-sheet-motion')).toMatch(
      /--tw-ease:\s*var\(--ease-out\)/
    );
  });
});

/**
 * Spec 2.6, V3-R7-1: once the last raw content moved onto `ModalShell` (the non-private model
 * disclosure, ruling 23), the primitive's `p-6 gap-4 sm:max-w-lg` defaults went, so a raw content
 * cannot drift into a near-miss dialog (24px insets, 512 wide). `rawDialogContent.sourceGuard`
 * holds the call sites; this holds the primitive.
 */
describe('DialogContent carries no inset, gap or width default', () => {
  it('renders a raw content with none of the three, and keeps the small-screen clamp', async () => {
    const user = userEvent.setup();
    render(<ControlledDialog />);
    await user.click(screen.getByRole('button', { name: 'Open dialog' }));
    const classes = Array.from(screen.getByRole('dialog').classList);
    expect(classes.filter((c) => /^(sm:)?(p|px|py|pt|pb|gap)-\d/.test(c))).toEqual([]);
    expect(classes.filter((c) => c.includes('max-w'))).toEqual(['max-w-[calc(100%-2rem)]']);
  });

  it('still takes a caller’s own inset, gap and width', async () => {
    render(
      <Dialog open>
        <DialogContent aria-describedby={undefined} className="gap-3 p-4 sm:max-w-[400px]">
          <DialogTitle>Sized by its caller</DialogTitle>
        </DialogContent>
      </Dialog>
    );
    expect(screen.getByRole('dialog')).toHaveClass('gap-3', 'p-4', 'sm:max-w-[400px]');
  });
});
