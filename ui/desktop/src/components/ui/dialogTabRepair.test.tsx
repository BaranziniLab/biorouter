import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Dialog, DialogContent, DialogTitle } from './dialog';

/**
 * The measured defect (Chromium, the **New schedule** dialog): twelve Tab
 * presses cycled Name → Browse → "Repeat every" → the dialog container, and
 * Cancel, "Create schedule", the × and the three time selects were never
 * focused.
 *
 * ⚠ **jsdom cannot reproduce the cause, and `userEvent.tab()` passes on the
 * unfixed code** — it computes the next focusable and focuses it directly, so
 * the window this bug lives in (the moment between `focusout` and `focusin`,
 * when `document.activeElement` is `<body>` and a `MutationObserver` microtask
 * gets to run) never opens. It was measured with Playwright against the real
 * component; what is pinned here is the *pattern* that window leaves behind — a
 * `focusout` that named its destination, immediately followed by the dialog
 * container taking focus instead — and the two cases the repair must not touch.
 */
function renderDialog() {
  render(
    <Dialog open>
      <DialogContent aria-describedby={undefined}>
        <DialogTitle>Schedule</DialogTitle>
        <button type="button">First</button>
        <button type="button">Second</button>
      </DialogContent>
    </Dialog>
  );
  return {
    container: screen.getByRole('dialog'),
    first: screen.getByRole('button', { name: 'First' }),
    second: screen.getByRole('button', { name: 'Second' }),
  };
}

/** The restore is deferred past the park loop, so a check has to be too. */
const settle = () => Promise.resolve();

/**
 * The three steps the browser really performs, in order: the control blurs, the
 * browser reports where the Tab was headed, and — in the window where nothing is
 * focused — Radix parks focus on the dialog. `blur()` first is what makes the
 * park realistic: it is only because focus is on `<body>` that `container.focus()`
 * raises no `focusout` of its own.
 */
function parkFocusOnDialog(
  container: HTMLElement,
  from: HTMLElement,
  headedFor: HTMLElement | null
) {
  from.blur();
  fireEvent.focusOut(from, { relatedTarget: headedFor });
  container.focus();
}

describe('dialog Tab repair', () => {
  it('hands focus back to the control the Tab was going to', async () => {
    const { container, first, second } = renderDialog();
    first.focus();

    parkFocusOnDialog(container, first, second);
    await settle();

    expect(document.activeElement).toBe(second);
  });

  it('answers a park that repeats, which is how the real one arrives', async () => {
    const { container, first, second } = renderDialog();
    first.focus();

    // Radix parks once per mutation record, and react-select's blur produces
    // four. A repair that restored inside the first park's `focusin` would be
    // overwritten by the rest and the user would still land on the dialog.
    parkFocusOnDialog(container, first, second);
    container.focus();
    container.focus();
    await settle();

    expect(document.activeElement).toBe(second);
  });

  it('leaves the park alone when the focused control really was removed', async () => {
    const { container, first } = renderDialog();
    first.focus();

    // A genuine removal drops focus to the document, so the browser has no
    // destination to report. Radix's park is the right answer here.
    parkFocusOnDialog(container, first, null);
    await settle();

    expect(document.activeElement).toBe(container);
  });

  it('does not chase a destination from an earlier task', async () => {
    const { container, first, second } = renderDialog();
    first.focus();
    first.blur();
    fireEvent.focusOut(first, { relatedTarget: second });

    // The Tab finished long ago; the dialog is focused later for its own
    // reasons — a click on its chrome, say.
    await new Promise((resolve) => setTimeout(resolve, 0));
    container.focus();
    await settle();

    expect(document.activeElement).toBe(container);
  });

  it('tells a screen reader the dialog is modal', () => {
    const { container } = renderDialog();
    expect(container).toHaveAttribute('aria-modal', 'true');
  });
});
