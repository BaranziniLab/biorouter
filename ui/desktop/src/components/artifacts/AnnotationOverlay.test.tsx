import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AnnotationOverlay, {
  ANNOTATION_MODIFIERS_HINT,
  ANNOTATION_PROMPT,
} from './AnnotationOverlay';

/**
 * The interaction is lifted from `Cmd+Shift+4`, and these tests pin the parts
 * that make it feel exact rather than approximate. jsdom has no layout, so the
 * overlay's own box is stubbed and every assertion is about the *arithmetic* —
 * which is where the modifier behaviour actually lives.
 */

beforeEach(() => {
  Element.prototype.setPointerCapture = vi.fn();
  Element.prototype.releasePointerCapture = vi.fn();
  Element.prototype.getBoundingClientRect = vi.fn(
    () =>
      ({
        left: 0,
        top: 0,
        width: 1000,
        height: 800,
        right: 1000,
        bottom: 800,
        x: 0,
        y: 0,
      }) as DOMRect
  );
});

function setup() {
  const onSelect = vi.fn();
  const onCancel = vi.fn();
  render(<AnnotationOverlay onSelect={onSelect} onCancel={onCancel} />);
  return { onSelect, onCancel, overlay: screen.getByTestId('annotation-overlay') };
}

const drag = (overlay: HTMLElement, from: [number, number], to: [number, number]) => {
  fireEvent.pointerDown(overlay, { button: 0, clientX: from[0], clientY: from[1], pointerId: 1 });
  fireEvent.pointerMove(overlay, { clientX: to[0], clientY: to[1], pointerId: 1 });
};

describe('selecting a region', () => {
  it('reports the dragged rectangle after hiding the capture chrome', async () => {
    const { onSelect, overlay } = setup();
    drag(overlay, [100, 100], [340, 280]);
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    expect(overlay).toHaveClass('invisible');
    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ x: 100, y: 100, width: 240, height: 180 })
    );
  });

  it('normalises a drag that goes up and to the left', async () => {
    const { onSelect, overlay } = setup();
    drag(overlay, [400, 400], [300, 250]);
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ x: 300, y: 250, width: 100, height: 150 })
    );
  });

  // The live readout is the detail that makes drag-to-crop feel precise; it is
  // the one thing every implementation of this gets asked for.
  it('shows the size while dragging', () => {
    const { overlay } = setup();
    drag(overlay, [10, 10], [210, 130]);
    expect(screen.getByTestId('annotation-dimensions')).toHaveTextContent('200 × 120');
  });

  it('ignores a click that is really a click', () => {
    const { onSelect, onCancel, overlay } = setup();
    drag(overlay, [50, 50], [52, 51]);
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    expect(onSelect).not.toHaveBeenCalled();
    // And stays in the mode. Reviewing a figure produces several notes, not
    // one, and exiting on a stray click is the most-complained-about detail in
    // every shipped version of this feature.
    expect(onCancel).not.toHaveBeenCalled();
  });
});

describe('the modifiers', () => {
  it('Shift constrains to a square, on the larger axis', async () => {
    const { onSelect, overlay } = setup();
    fireEvent.pointerDown(overlay, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.keyDown(window, { key: 'Shift' });
    fireEvent.pointerMove(overlay, { clientX: 300, clientY: 180, pointerId: 1 });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ x: 100, y: 100, width: 200, height: 200 })
    );
  });

  it('Option sizes from the centre', async () => {
    const { onSelect, overlay } = setup();
    fireEvent.pointerDown(overlay, { button: 0, clientX: 500, clientY: 400, pointerId: 1 });
    fireEvent.keyDown(window, { key: 'Alt' });
    fireEvent.pointerMove(overlay, { clientX: 600, clientY: 450, pointerId: 1 });
    fireEvent.pointerUp(overlay, { pointerId: 1 });
    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({ x: 400, y: 350, width: 200, height: 100 })
    );
  });

  it('Space moves the marquee without resizing it', async () => {
    const { onSelect, overlay } = setup();
    drag(overlay, [100, 100], [300, 250]);
    fireEvent.keyDown(window, { key: ' ' });
    fireEvent.pointerMove(overlay, { clientX: 340, clientY: 300, pointerId: 1 });
    fireEvent.keyUp(window, { key: ' ' });
    fireEvent.pointerUp(overlay, { pointerId: 1 });

    await waitFor(() => expect(onSelect).toHaveBeenCalledOnce());
    const region = onSelect.mock.calls[0][0];
    expect(region.width).toBe(200);
    expect(region.height).toBe(150);
    expect(region.x).toBe(140);
    expect(region.y).toBe(150);
  });

  it('Escape always cancels', () => {
    const { onCancel, overlay } = setup();
    drag(overlay, [100, 100], [300, 250]);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

describe('the pill over the preview', () => {
  /**
   * One line: the action and the way out (owner, 2026-11-13). The modifier keys moved to the
   * camera button's tooltip, because the overlay takes no pointer events and so cannot hold a
   * hover of its own.
   */
  it('says the action and the way out, and leaves the modifier keys to the camera button', () => {
    setup();
    const pill = screen.getByText(ANNOTATION_PROMPT);
    expect(pill).toHaveTextContent('Drag to select a region');
    expect(pill).toHaveTextContent('Esc to cancel');
    expect(pill).not.toHaveTextContent(/Shift|Option|Space/);
    // Nothing is lost: every key the overlay honours is still said, on the trigger.
    for (const key of ['Shift', 'Option', 'Space']) {
      expect(ANNOTATION_MODIFIERS_HINT).toContain(key);
    }
  });
});
