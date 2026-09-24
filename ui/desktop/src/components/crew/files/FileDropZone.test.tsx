import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Composer } from '../composer/Composer';
import { channelReady, general, installDaemon, renderCrew } from '../integration/harness';
import { installResizeObserverStub } from '../test/crewTestUtils';
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
  clearPublishedTransfers: vi.fn(),
}));

// The real layout, for the channel stage's own zone (see `integration/harness.tsx`).
vi.mock('../crewApi', async () => {
  const actual = await vi.importActual<typeof import('../crewApi')>('../crewApi');
  return { ...actual, crewHttp: vi.fn(), crewRequest: vi.fn(), observeCrew: vi.fn() };
});
const config = vi.hoisted(() => ({
  getProviders: async () => [{ name: 'fixture-provider', is_configured: true }],
  read: async () => '',
  getProviderModels: async () => ['fixture-model'],
}));
vi.mock('../../ConfigContext', async () => {
  const actual = await vi.importActual<typeof import('../../ConfigContext')>('../../ConfigContext');
  return { ...actual, useConfig: () => config };
});
vi.mock('../CrewAuthentication', () => ({ default: () => <div /> }));

installResizeObserverStub();

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

  it('clears the overlay when the drag ends somewhere else', () => {
    render(
      <CrewTestProvider controller={crewTestController()}>
        <CrewFileDropZone>
          <div data-testid="timeline">messages</div>
          <Composer />
        </CrewFileDropZone>
        <div data-testid="elsewhere">elsewhere</div>
      </CrewTestProvider>
    );
    const timeline = screen.getByTestId('timeline');
    fireEvent.dragEnter(timeline, { dataTransfer: files('counts.csv') });
    fireEvent.dragEnter(screen.getByLabelText('Message #general'), {
      dataTransfer: files('counts.csv'),
    });
    expect(screen.getByText('Drop to share in #general')).toBeInTheDocument();
    fireEvent.drop(screen.getByTestId('elsewhere'), { dataTransfer: files('counts.csv') });
    expect(screen.queryByText('Drop to share in #general')).toBeNull();
    expect(mocks.beginTransfer).not.toHaveBeenCalled();
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

describe('the drop overlay (Q2-60)', () => {
  it('tints the conversation instead of blanking it, with its words on their own label', () => {
    render(
      <CrewTestProvider controller={crewTestController()}>
        <CrewFileDropZone>
          <div data-testid="timeline">messages</div>
          <Composer />
        </CrewFileDropZone>
      </CrewTestProvider>
    );
    fireEvent.dragEnter(screen.getByTestId('timeline'), { dataTransfer: files('counts.csv') });
    const overlay = screen.getByTestId('crew-drop-overlay');
    expect(
      within(overlay).getByText('Drop to share in #general').closest('.crew-drop-overlay-label')
    ).not.toBeNull();
    // The messages stay in the document and in view under the tint.
    expect(screen.getByTestId('timeline')).toBeVisible();

    const css = readFileSync(join(__dirname, 'files.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    const rule = (selector: string) => {
      const at = css.indexOf(`${selector} {`);
      return css.slice(at, css.indexOf('}', at));
    };
    // Translucent: the canvas mixed with transparency, never an opaque ground.
    expect(rule('.crew-drop-overlay')).toMatch(
      /background-color: color-mix\(in srgb, var\(--background-canvas\) \d+%, transparent\);/
    );
    expect(rule('.crew-drop-overlay')).not.toMatch(/background-color: var\(/);
    expect(rule('.crew-drop-overlay-label')).toMatch(
      /background-color: var\(--background-default\);/
    );
  });
});

describe('the channel stage’s drop zone (T-26)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.beginTransfer.mockReset().mockResolvedValue(null);
    mocks.listTransfers.mockReset().mockResolvedValue([]);
    installDaemon();
  });

  it('takes a file dropped on the messages, through the composer’s one upload path', async () => {
    renderCrew();
    await channelReady();
    // One zone for the whole channel body: the composer registered with it.
    expect(document.querySelectorAll('[data-drop-zone="true"]')).toHaveLength(1);
    const log = screen.getByRole('log', { name: 'general messages' });
    const zone = log.closest('[data-drop-zone="true"]') as HTMLElement;
    expect(zone).not.toBeNull();
    expect(zone).toContainElement(screen.getByRole('textbox', { name: 'Message #general' }));

    fireEvent.dragEnter(log, { dataTransfer: files('counts.csv') });
    expect(within(zone).getByTestId('crew-drop-overlay')).toHaveTextContent(
      'Drop to share in #general'
    );
    fireEvent.drop(log, { dataTransfer: files('counts.csv') });
    expect(screen.queryByTestId('crew-drop-overlay')).toBeNull();
    // Through the secure picker, as a drop on the composer does; no path goes anywhere.
    await waitFor(() =>
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: expect.any(String),
        connection_id: expect.any(String),
        channel_id: general.id,
        direction: 'upload',
      })
    );
    expect(Object.keys(mocks.beginTransfer.mock.calls[0][0]).sort()).toEqual([
      'channel_id',
      'connection_id',
      'direction',
      'expected_mode',
    ]);
  });

  describe('with the native share confirmation (D-DROP)', () => {
    const originalElectron = (window as { electron?: unknown }).electron;
    const share = vi.fn();
    beforeEach(() => {
      share.mockReset().mockResolvedValue({
        outcome: 'shared',
        capability_id: 'cap-1',
        name: 'counts.csv',
        size: 1,
      });
      (window as { electron?: unknown }).electron = {
        crewShareDroppedFile: share,
        crewSelectTransferFile: vi.fn(),
        getPathForFile: () => '/Users/dave/counts.csv',
      };
    });
    afterEach(() => {
      (window as { electron?: unknown }).electron = originalElectron;
    });

    it('asks once for a file dropped on the messages, then uploads it to this channel', async () => {
      renderCrew();
      await channelReady();
      const log = screen.getByRole('log', { name: 'general messages' });
      const dropped = files('counts.csv');
      fireEvent.dragEnter(log, { dataTransfer: dropped });
      fireEvent.drop(log, { dataTransfer: dropped });
      await waitFor(() => expect(share).toHaveBeenCalledTimes(1));
      // The File the drop carried, and where it goes; the preload finds the path itself.
      expect(share.mock.calls[0][0]).toBe(dropped.files[0]);
      expect(share.mock.calls[0][1]).toMatchObject({
        connectionId: expect.any(String),
        channelId: general.id,
        channelName: 'general',
      });
      expect(Object.keys(share.mock.calls[0][1]).sort()).toEqual([
        'channelId',
        'channelName',
        'connectionId',
        'expectedMode',
        'workspaceName',
      ]);
      await waitFor(() =>
        expect(mocks.beginTransfer).toHaveBeenCalledWith(
          {
            expected_mode: expect.any(String),
            connection_id: expect.any(String),
            channel_id: general.id,
            direction: 'upload',
          },
          { capability_id: 'cap-1', name: 'counts.csv', size: 1 }
        )
      );
    });
  });
});
