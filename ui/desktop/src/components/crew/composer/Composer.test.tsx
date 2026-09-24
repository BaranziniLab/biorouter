import { act, createEvent, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { crewActionCopy } from '../state/copy';
import type { CrewController, SurfaceResetListener } from '../state/types';
import { crewTestController, CrewTestProvider, testChannel } from '../files/crewTestController';
import { Composer } from './Composer';
import { composerCopy } from './copy';

const mocks = vi.hoisted(() => ({
  beginTransfer: vi.fn(),
  listTransfers: vi.fn(),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
}));

vi.mock('../crewTransfers', () => ({
  beginTransfer: mocks.beginTransfer,
  listTransfers: mocks.listTransfers,
  pauseTransfer: mocks.pauseTransfer,
  resumeTransfer: mocks.resumeTransfer,
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
}));

function renderComposer(overrides: Partial<CrewController> = {}, note?: ReactNode) {
  const controller = crewTestController(overrides);
  const view = render(
    <CrewTestProvider controller={controller}>
      <Composer note={note} />
    </CrewTestProvider>
  );
  const rerender = (next: Partial<CrewController>) =>
    view.rerender(
      <CrewTestProvider controller={crewTestController({ ...overrides, ...next })}>
        <Composer note={note} />
      </CrewTestProvider>
    );
  return { ...view, controller, rerenderWith: rerender };
}

const withBody = (body: string) => ({ draft: { body, attachments: [], references: [] } });

describe('Crew composer', () => {
  beforeEach(() => {
    mocks.beginTransfer.mockReset();
    mocks.listTransfers.mockReset().mockResolvedValue([]);
    mocks.pauseTransfer.mockReset();
    mocks.resumeTransfer.mockReset();
  });

  it('names the textarea and its placeholder for the channel (pinned)', () => {
    renderComposer();
    const input = screen.getByLabelText('Message #general');
    expect(input.tagName).toBe('TEXTAREA');
    expect(input).toHaveAttribute('placeholder', 'Message #general');
    expect(input).toHaveAttribute('rows', '1');
    expect(input).not.toBeDisabled();
  });

  describe('keys', () => {
    it('does not send or prevent Shift+Enter, so it inserts a newline', () => {
      const send = vi.fn(async () => undefined);
      renderComposer({ ...withBody('line one'), send });
      const input = screen.getByLabelText('Message #general');
      const shiftEnter = createEvent.keyDown(input, {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        shiftKey: true,
      });
      fireEvent(input, shiftEnter);
      expect(shiftEnter.defaultPrevented).toBe(false);
      expect(send).not.toHaveBeenCalled();
    });

    it('ignores Enter while an input method is composing', () => {
      const send = vi.fn(async () => undefined);
      renderComposer({ ...withBody('にほん'), send });
      const input = screen.getByLabelText('Message #general');
      const composing = createEvent.keyDown(input, {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        isComposing: true,
      });
      fireEvent(input, composing);
      const process = createEvent.keyDown(input, { key: 'Enter', code: 'Enter', keyCode: 229 });
      fireEvent(input, process);
      expect(composing.defaultPrevented).toBe(false);
      expect(process.defaultPrevented).toBe(false);
      expect(send).not.toHaveBeenCalled();
    });

    it('sends once on Enter and never on a held key repeat', () => {
      const send = vi.fn(async () => undefined);
      renderComposer({ ...withBody('hello'), send });
      const input = screen.getByLabelText('Message #general');
      const enter = createEvent.keyDown(input, { key: 'Enter', code: 'Enter', keyCode: 13 });
      fireEvent(input, enter);
      expect(enter.defaultPrevented).toBe(true);
      expect(send).toHaveBeenCalledTimes(1);

      const held = createEvent.keyDown(input, {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
        repeat: true,
      });
      fireEvent(input, held);
      expect(held.defaultPrevented).toBe(true);
      expect(send).toHaveBeenCalledTimes(1);
    });

    it('writes every keystroke to the controller draft', () => {
      const setBody = vi.fn();
      renderComposer({ setBody });
      fireEvent.change(screen.getByLabelText('Message #general'), {
        target: { value: 'draft text' },
      });
      expect(setBody).toHaveBeenCalledWith('draft text');
    });
  });

  describe('Send', () => {
    it('is quiet and disabled until there is content, then the accent', () => {
      const { rerenderWith } = renderComposer();
      const send = screen.getByRole('button', { name: 'Send message' });
      expect(send).toHaveAttribute('data-variant', 'secondary');
      expect(send).toBeDisabled();

      rerenderWith(withBody('something'));
      const armed = screen.getByRole('button', { name: 'Send message' });
      expect(armed).toBe(send);
      expect(armed).toHaveAttribute('data-variant', 'default');
      expect(armed).toBeEnabled();
    });

    it('counts attachments and server paths as content', () => {
      renderComposer({
        draft: { body: '', attachments: [], references: [{ id: 'ref-1', label: 'Results' }] },
      });
      expect(screen.getByRole('button', { name: 'Send message' })).toHaveAttribute(
        'data-variant',
        'default'
      );
    });

    it('stays the same node while posting and ignores presses (single flight)', async () => {
      const send = vi.fn(async () => undefined);
      const { rerenderWith } = renderComposer({ ...withBody('post me'), send });
      const button = screen.getByRole('button', { name: 'Send message' });
      await userEvent.setup().click(button);
      expect(send).toHaveBeenCalledTimes(1);

      rerenderWith({ ...withBody('post me'), send, isPending: (key) => key === 'send' });
      const posting = screen.getByRole('button', { name: 'Send message' });
      expect(posting).toBe(button);
      expect(posting).toHaveAttribute('aria-busy', 'true');
      fireEvent.click(posting);
      expect(send).toHaveBeenCalledTimes(1);

      rerenderWith({ ...withBody(''), send });
      expect(screen.getByRole('button', { name: 'Send message' })).toBe(button);
    });

    it('keeps the text read-only, not disabled, while posting, and focus in it after a press', async () => {
      const send = vi.fn(async () => undefined);
      const { rerenderWith } = renderComposer({ ...withBody('post me'), send });
      const input = screen.getByLabelText('Message #general');
      await userEvent.setup().click(screen.getByRole('button', { name: 'Send message' }));
      expect(input).toHaveFocus();

      rerenderWith({ ...withBody('post me'), send, isPending: (key) => key === 'send' });
      expect(input).toHaveAttribute('readonly');
      expect(input).not.toBeDisabled();
      expect(input).toHaveFocus();

      rerenderWith({ ...withBody(''), send });
      expect(input).not.toHaveAttribute('readonly');
    });
  });

  describe('stand-ins for the card', () => {
    it('does not mount the textarea without a verified snapshot (C13)', () => {
      const { rerenderWith } = renderComposer({
        ...withBody('kept'),
        snapshot: null,
        channel: null,
      });
      expect(screen.queryByLabelText('Message #general')).toBeNull();
      expect(screen.queryByRole('textbox')).toBeNull();
      expect(screen.getByText(composerCopy.verifying)).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Send message' })).toBeNull();
      expect(screen.queryByRole('button', { name: 'Ask my agent' })).toBeNull();

      rerenderWith({ snapshot: crewTestController().snapshot, channel: testChannel });
      expect(screen.getByLabelText('Message #general')).toHaveValue('kept');
      expect(screen.queryByText(composerCopy.verifying)).toBeNull();
    });

    it('replaces the card with a bar in an archived channel', () => {
      renderComposer({ channel: { ...testChannel, archived: true } });
      expect(screen.getByText('This channel is archived.')).toBeInTheDocument();
      expect(screen.queryByLabelText('Message #general')).toBeNull();
      expect(screen.queryByRole('button', { name: 'Attach' })).toBeNull();
    });
  });

  describe('chips', () => {
    it('removes a file and a server path by their pinned names', async () => {
      const removeAttachment = vi.fn();
      const removeReference = vi.fn();
      renderComposer({
        draft: {
          body: '',
          attachments: [{ id: 'blob-1', name: 'counts.csv' }],
          references: [{ id: 'ref-1', label: 'Remote results' }],
        },
        removeAttachment,
        removeReference,
      });
      const user = userEvent.setup();
      await user.click(screen.getByRole('button', { name: 'Remove counts.csv' }));
      expect(removeAttachment).toHaveBeenCalledWith('blob-1');
      await user.click(
        screen.getByRole('button', { name: 'Remove remote reference Remote results' })
      );
      expect(removeReference).toHaveBeenCalledWith('ref-1');
      expect(screen.getByText('counts.csv')).toBeInTheDocument();
      expect(screen.getByText('Remote results')).toBeInTheDocument();
    });

    it('renders no chips row for an empty draft', () => {
      renderComposer();
      expect(screen.queryByRole('list', { name: 'Attachments' })).toBeNull();
    });

    it('shows an upload on its way with its percent and a Pause control', async () => {
      mocks.listTransfers.mockResolvedValue([
        {
          id: 'transfer-1',
          request_id: 'request-1',
          connection_id: 'connection-1',
          channel_id: 'channel-1',
          direction: 'upload',
          name: 'counts.csv',
          size: 1000,
          sha256: '',
          offset: 420,
          blob_id: null,
          state: 'uploading',
          error: null,
        },
      ]);
      mocks.pauseTransfer.mockResolvedValue({});
      renderComposer();
      expect(await screen.findByText('42%')).toBeInTheDocument();
      await userEvent.setup().click(screen.getByRole('button', { name: 'Pause counts.csv' }));
      expect(mocks.pauseTransfer).toHaveBeenCalledWith('transfer-1');
    });
  });

  describe('the note slot', () => {
    it('shows the send failure once, the daemon words in their own node', () => {
      renderComposer({ error: { message: 'send failed', source: 'composer' } });
      const alert = screen.getByRole('alert');
      expect(alert).toHaveTextContent('Couldn’t send. send failed');
      expect(screen.getAllByText('send failed')).toHaveLength(1);
      expect(screen.getAllByRole('alert')).toHaveLength(1);
    });

    it('leaves an error that belongs to another surface alone', () => {
      renderComposer({
        error: { message: 'start failed', source: 'pane:agent' },
        errorSlotFor: (source) => source === 'pane:agent',
      });
      expect(screen.queryByText('start failed')).toBeNull();
      expect(screen.queryByRole('alert')).toBeNull();
    });

    it('turns the kept-upload-record sentence into the sent warning', () => {
      renderComposer({
        error: { message: crewActionCopy.sendTransferRecordKept, source: 'composer' },
      });
      expect(screen.getByRole('status')).toHaveTextContent(composerCopy.postedMetadata);
      expect(screen.queryByText(composerCopy.sendErrorLead)).toBeNull();
    });

    it('shows the layout note only while nothing more urgent is showing', () => {
      const note = <p>Allow this chat?</p>;
      const { rerenderWith } = renderComposer({}, note);
      expect(screen.getByText('Allow this chat?')).toBeInTheDocument();
      rerenderWith({ error: { message: 'send failed', source: 'composer' } });
      expect(screen.queryByText('Allow this chat?')).toBeNull();
    });

    it('registers as the composer error slot while mounted', () => {
      const unregister = vi.fn();
      const registerErrorSlot = vi.fn(() => unregister);
      const { unmount } = renderComposer({ registerErrorSlot });
      expect(registerErrorSlot).toHaveBeenCalledWith('composer');
      unmount();
      expect(unregister).toHaveBeenCalled();
    });
  });

  describe('uploads and the verified scope', () => {
    const finished = {
      id: 'transfer-7',
      request_id: 'request-7',
      connection_id: 'connection-1',
      channel_id: 'channel-1',
      direction: 'upload',
      name: 'late.csv',
      size: 1,
      sha256: '',
      offset: 1,
      blob_id: 'blob-7',
      state: 'completed',
      error: null,
    };

    async function startUpload() {
      const user = userEvent.setup();
      await user.click(screen.getByRole('button', { name: 'Attach' }));
      await user.click(await screen.findByRole('menuitem', { name: 'Upload a file…' }));
    }

    it('drops an upload started under a privacy scope that has since changed', async () => {
      const addAttachment = vi.fn();
      let transfers: unknown[] = [];
      mocks.listTransfers.mockImplementation(async () => transfers);
      mocks.beginTransfer.mockResolvedValue({ id: 'transfer-7' });
      const { rerenderWith } = renderComposer({ addAttachment });
      await startUpload();
      expect(mocks.beginTransfer).toHaveBeenCalledTimes(1);

      rerenderWith({
        addAttachment,
        observedPrivacy: {
          connectionId: 'connection-1',
          mode: 'private',
          institutionId: 'ucsf',
          policyEpoch: 2,
        },
      });
      transfers = [finished];
      await act(async () => {
        const { refreshCrewTransfers } = await import('../files/useCrewTransfers');
        await refreshCrewTransfers('connection-1');
      });
      expect(addAttachment).not.toHaveBeenCalled();
    });

    it('keeps an upload across a refresh that verifies the same scope again', async () => {
      const addAttachment = vi.fn();
      let transfers: unknown[] = [];
      mocks.listTransfers.mockImplementation(async () => transfers);
      mocks.beginTransfer.mockResolvedValue({ id: 'transfer-7' });
      const { rerenderWith } = renderComposer({ addAttachment });
      await startUpload();

      rerenderWith({ addAttachment, snapshot: null, channel: null });
      expect(screen.queryByLabelText('Message #general')).toBeNull();
      rerenderWith({ addAttachment });
      transfers = [finished];
      await act(async () => {
        const { refreshCrewTransfers } = await import('../files/useCrewTransfers');
        await refreshCrewTransfers('connection-1');
      });
      expect(addAttachment).toHaveBeenCalledWith({ id: 'blob-7', name: 'late.csv' });
    });
  });

  describe('controls', () => {
    it('opens the agent pane from Ask my agent (pinned)', async () => {
      const openPane = vi.fn();
      renderComposer({ openPane });
      await userEvent.setup().click(screen.getByRole('button', { name: 'Ask my agent' }));
      expect(openPane).toHaveBeenCalledWith({ mode: 'agent' });
    });

    it('offers upload and server path from a real Attach menu', async () => {
      const openDialog = vi.fn();
      renderComposer({ openDialog });
      const user = userEvent.setup();
      const attach = screen.getByRole('button', { name: 'Attach' });
      expect(attach).toHaveAttribute('aria-haspopup', 'menu');
      await user.click(attach);
      const menu = await screen.findByRole('menu');
      expect(within(menu).getByRole('menuitem', { name: 'Upload a file…' })).toBeInTheDocument();
      await user.click(within(menu).getByRole('menuitem', { name: 'Share a server path…' }));
      expect(openDialog).toHaveBeenCalledWith({ kind: 'share-path' });
    });

    it('stops watching an upload when the verified view is cleared', async () => {
      let listener: SurfaceResetListener | undefined;
      const addAttachment = vi.fn();
      let transfers: unknown[] = [];
      mocks.listTransfers.mockImplementation(async () => transfers);
      mocks.beginTransfer.mockResolvedValue({ id: 'transfer-9' });
      renderComposer({
        addAttachment,
        subscribeSurfaceReset: (next) => {
          listener = next;
          return () => undefined;
        },
      });
      const user = userEvent.setup();
      await user.click(screen.getByRole('button', { name: 'Attach' }));
      await user.click(await screen.findByRole('menuitem', { name: 'Upload a file…' }));
      expect(mocks.beginTransfer).toHaveBeenCalledTimes(1);
      await act(async () => listener?.('protected-cleared'));
      transfers = [
        {
          id: 'transfer-9',
          connection_id: 'connection-1',
          channel_id: 'channel-1',
          direction: 'upload',
          name: 'late.csv',
          size: 1,
          offset: 1,
          blob_id: 'blob-9',
          state: 'completed',
          error: null,
        },
      ];
      mocks.pauseTransfer.mockResolvedValue({});
      // Any action asks for a fresh list; the finished upload must not reach the draft.
      await act(async () => {
        const { refreshCrewTransfers } = await import('../files/useCrewTransfers');
        await refreshCrewTransfers('connection-1');
      });
      expect(addAttachment).not.toHaveBeenCalled();
    });
  });
});
