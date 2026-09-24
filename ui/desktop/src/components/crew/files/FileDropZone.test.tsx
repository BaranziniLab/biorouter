import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { Composer } from '../composer/Composer';
import { crewTestController, CrewTestProvider } from './crewTestController';
import { CrewFileDropZone } from './FileDropZone';

const mocks = vi.hoisted(() => ({ beginTransfer: vi.fn(), listTransfers: vi.fn() }));

vi.mock('../crewTransfers', () => ({
  beginTransfer: mocks.beginTransfer,
  listTransfers: mocks.listTransfers,
  forgetTransfer: vi.fn(),
  pauseTransfer: vi.fn(),
  previewAttachment: vi.fn(),
  resumeTransfer: vi.fn(),
}));

const files = (name: string) => ({
  types: ['Files'],
  files: [new File(['x'], name)],
  items: [{ kind: 'file', webkitGetAsEntry: () => ({ isDirectory: false }) }],
  dropEffect: 'none',
});

describe('CrewFileDropZone', () => {
  beforeEach(() => {
    mocks.beginTransfer.mockReset().mockResolvedValue(null);
    mocks.listTransfers.mockReset().mockResolvedValue([]);
  });

  it('makes the whole channel a drop zone for the composer inside it', async () => {
    render(
      <CrewTestProvider controller={crewTestController()}>
        <CrewFileDropZone>
          <div data-testid="timeline">messages</div>
          <Composer />
        </CrewFileDropZone>
      </CrewTestProvider>
    );
    const timeline = screen.getByTestId('timeline');
    // The composer registered with the channel's zone instead of making its own.
    expect(document.querySelectorAll('[data-drop-zone="true"]')).toHaveLength(1);

    fireEvent.dragEnter(timeline, { dataTransfer: files('counts.csv') });
    expect(screen.getByText('Drop to share in #general')).toBeInTheDocument();
    fireEvent.dragLeave(timeline, { dataTransfer: files('counts.csv') });
    expect(screen.queryByText('Drop to share in #general')).toBeNull();

    fireEvent.drop(timeline, { dataTransfer: files('counts.csv') });
    await waitFor(() =>
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: 'private',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      })
    );
  });

  it('takes nothing, and shows nothing, while no surface can take files', () => {
    render(
      <CrewFileDropZone>
        <div data-testid="timeline">messages</div>
      </CrewFileDropZone>
    );
    const timeline = screen.getByTestId('timeline');
    const over = fireEvent.dragOver(timeline, { dataTransfer: files('counts.csv') });
    // Still claimed, so the window never navigates to the dropped file.
    expect(over).toBe(false);
    fireEvent.dragEnter(timeline, { dataTransfer: files('counts.csv') });
    expect(screen.queryByTestId('crew-drop-overlay')).toBeNull();
    const dropped = fireEvent.drop(timeline, { dataTransfer: files('counts.csv') });
    expect(dropped).toBe(false);
  });

  it('stops offering the channel once the composer inside cannot take files', () => {
    const { rerender } = render(
      <CrewTestProvider controller={crewTestController()}>
        <CrewFileDropZone>
          <div data-testid="timeline">messages</div>
          <Composer />
        </CrewFileDropZone>
      </CrewTestProvider>
    );
    rerender(
      <CrewTestProvider controller={crewTestController({ snapshot: null, channel: null })}>
        <CrewFileDropZone>
          <div data-testid="timeline">messages</div>
          <Composer />
        </CrewFileDropZone>
      </CrewTestProvider>
    );
    fireEvent.dragEnter(screen.getByTestId('timeline'), { dataTransfer: files('counts.csv') });
    expect(screen.queryByText(/Drop to share/)).toBeNull();
  });
});
