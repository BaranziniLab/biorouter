import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { Dropzone, useFileDrop } from './Dropzone';
import { knowledgeCopy } from '../copy';

beforeAll(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

function renderDropzone() {
  const onPasteText = vi.fn();
  render(<Dropzone onFiles={vi.fn()} onPathPickRequested={vi.fn()} onPasteText={onPasteText} />);
  return { onPasteText };
}

const chooser = () => screen.queryByRole('dialog', { name: knowledgeCopy.ingest.chooserTitle });

/**
 * The dropzone is a drop target that also holds two controls (the InfoTip and "Paste text"). A
 * press anywhere on the box opens the chooser, but a press on one of those must do its own job
 * and nothing else.
 */
describe('Dropzone', () => {
  it('opens the chooser from its label button and from the box around it', async () => {
    const user = userEvent.setup();
    renderDropzone();
    expect(chooser()).toBeNull();

    await user.click(screen.getByRole('button', { name: knowledgeCopy.ingest.dropButtonName }));
    expect(chooser()).toBeInTheDocument();
    expect(screen.getByText(knowledgeCopy.ingest.chooserSubtitle)).toBeInTheDocument();
  });

  it('names its button for what a press does, and keeps the words a person sees first', async () => {
    const user = userEvent.setup();
    renderDropzone();
    const { dropLabel, dropButtonName } = knowledgeCopy.ingest;
    const button = screen.getByRole('button', { name: dropButtonName });

    // The visible words stay, and they open the name, so saying them still reaches the button.
    expect(button).toHaveTextContent(dropLabel);
    expect(dropButtonName.startsWith(dropLabel)).toBe(true);
    // A press opens the chooser, so the name says so, not only the drag.
    expect(dropButtonName).not.toBe(dropLabel);
    expect(dropButtonName).toMatch(/\bchoose\b/i);

    // From the keyboard: Enter on the button opens the chooser the name promises.
    button.focus();
    await user.keyboard('{Enter}');
    expect(chooser()).toBeInTheDocument();
  });

  it('opens the chooser when the empty part of the box is pressed', async () => {
    const user = userEvent.setup();
    renderDropzone();
    await user.click(screen.getByTestId('knowledge-ingest-dropzone'));
    expect(chooser()).toBeInTheDocument();
  });

  it('keeps Paste text to itself: it summons the paste box and opens no chooser', async () => {
    const user = userEvent.setup();
    const { onPasteText } = renderDropzone();
    await user.click(screen.getByTestId('knowledge-ingest-paste-text'));
    expect(onPasteText).toHaveBeenCalledTimes(1);
    expect(chooser()).toBeNull();
  });

  it('keeps the InfoTip to itself, both its glyph and the floating text', async () => {
    const user = userEvent.setup();
    renderDropzone();

    await user.click(
      screen.getByRole('button', { name: `About ${knowledgeCopy.ingest.dropInfoLabel}` })
    );
    const tip = document.querySelector('[data-slot="info-tip-content"]');
    expect(tip).toHaveTextContent(knowledgeCopy.ingest.dropInfo);
    expect(chooser()).toBeNull();

    // The text is portaled out of the box, but React still bubbles its click through the
    // dropzone, which must not read it as a press on the drop target.
    fireEvent.click(tip!);
    expect(chooser()).toBeNull();
  });
});

/**
 * V3-12: the box is as tall as its content, so the drop is the whole rail's. A file let go below
 * the box (or on a staged row) must still stage, and the box is where the answer is drawn.
 */
function Rail({ onFiles }: { onFiles: (files: unknown[]) => void }) {
  const { dragging, dropProps } = useFileDrop(onFiles);
  return (
    <div data-testid="rail" {...dropProps}>
      <Dropzone
        dragging={dragging}
        onFiles={onFiles}
        onPathPickRequested={vi.fn()}
        onPasteText={vi.fn()}
      />
      <div data-testid="below-the-box">staged list</div>
      <textarea aria-label="Pasted text" />
    </div>
  );
}

const file = new File(['x'], 'notes.md', { type: 'text/markdown' });
const transfer = (files: File[] = []) => ({
  dataTransfer: { files, getData: () => '', types: files.length ? ['Files'] : [] },
});

describe('Dropzone as the rail’s drop target', () => {
  it('takes a file let go anywhere in the rail, not only on the box', () => {
    const onFiles = vi.fn();
    render(<Rail onFiles={onFiles} />);
    const below = screen.getByTestId('below-the-box');

    fireEvent.dragEnter(below, transfer([file]));
    expect(screen.getByTestId('knowledge-ingest-dropzone')).toHaveAttribute(
      'data-dragging',
      'true'
    );

    fireEvent.drop(below, transfer([file]));
    expect(onFiles).toHaveBeenCalledWith([{ file }]);
    expect(screen.getByTestId('knowledge-ingest-dropzone')).not.toHaveAttribute('data-dragging');
  });

  it('stays lit while the pointer moves between the rail’s children, and goes dark on leaving', () => {
    render(<Rail onFiles={vi.fn()} />);
    const box = screen.getByTestId('knowledge-ingest-dropzone');
    const below = screen.getByTestId('below-the-box');

    fireEvent.dragEnter(box, transfer([file]));
    fireEvent.dragEnter(below, transfer([file]));
    fireEvent.dragLeave(box, transfer([file]));
    expect(box).toHaveAttribute('data-dragging', 'true');

    fireEvent.dragLeave(below, transfer([file]));
    expect(box).not.toHaveAttribute('data-dragging');
  });

  it('leaves a text field alone, so text can still be dropped into the paste box', () => {
    const onFiles = vi.fn();
    render(<Rail onFiles={onFiles} />);
    const field = screen.getByRole('textbox', { name: 'Pasted text' });

    // A drag over a field claims nothing: the browser's own insertion is not cancelled.
    expect(fireEvent.dragEnter(field, transfer())).toBe(true);
    expect(screen.getByTestId('knowledge-ingest-dropzone')).not.toHaveAttribute('data-dragging');
    expect(fireEvent.drop(field, transfer())).toBe(true);
    expect(onFiles).not.toHaveBeenCalled();
  });
});
