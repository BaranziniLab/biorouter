import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { Dropzone } from './Dropzone';
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

    await user.click(screen.getByRole('button', { name: knowledgeCopy.ingest.dropLabel }));
    expect(chooser()).toBeInTheDocument();
    expect(screen.getByText(knowledgeCopy.ingest.chooserSubtitle)).toBeInTheDocument();
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
