import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, useState, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { crewActionCopy } from '../state/copy';
import type { CrewController, CrewDraft, SurfaceResetListener } from '../state/types';
import { AttachmentIndexProvider, useAttachmentIndex } from '../files/attachmentIndex';
import { filesCopy } from '../files/copy';
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

/**
 * A composer over a controller that keeps its draft, like the real one: typing writes the
 * body, and `send` runs the override (which may leave the draft alone, as a pending or refused
 * send does) or else clears what was sent.
 */
function StatefulComposer({
  overrides = {},
  note,
  initialBody = '',
}: {
  overrides?: Partial<CrewController>;
  note?: ReactNode;
  initialBody?: string;
}) {
  const [draft, setDraft] = useState<CrewDraft>({
    body: initialBody,
    attachments: [],
    references: [],
  });
  const controller = crewTestController({
    draft,
    setBody: (body) => setDraft((current) => ({ ...current, body })),
    send: async () => setDraft({ body: '', attachments: [], references: [] }),
    ...overrides,
  });
  return (
    <CrewTestProvider controller={controller}>
      <Composer note={note} />
    </CrewTestProvider>
  );
}

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
      await user.click(screen.getByRole('button', { name: 'Remove counts.csv from this message' }));
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

    it('names what × removes, and says the uploaded copy stays on the server (Q3-14)', async () => {
      renderComposer({
        connection: {
          id: 'connection-1',
          name: 'Lab',
          ssh_target: 'alice@52.33.141.141',
          server_label: 'lab-server',
        } as unknown as CrewController['connection'],
        draft: { body: '', attachments: [{ id: 'blob-1', name: 'counts.csv' }], references: [] },
      });
      expect(composerCopy.removeFile('counts.csv')).toBe('Remove counts.csv from this message');
      const remove = screen.getByRole('button', { name: 'Remove counts.csv from this message' });
      await userEvent.setup().hover(remove);
      expect(await screen.findByRole('tooltip')).toHaveTextContent(
        'Removes it from this message. The copy already uploaded stays on lab-server.'
      );
    });

    it('says once, under a finished file, that Send is what shares it (Q3-14)', () => {
      const { rerenderWith } = renderComposer({
        draft: { body: '', attachments: [{ id: 'blob-1', name: 'counts.csv' }], references: [] },
      });
      expect(screen.getAllByText('Press Send to share it.')).toHaveLength(1);
      rerenderWith({
        draft: {
          body: '',
          attachments: [
            { id: 'blob-1', name: 'counts.csv' },
            { id: 'blob-2', name: 'plate.csv' },
          ],
          references: [],
        },
      });
      expect(screen.queryByText('Press Send to share it.')).toBeNull();
      expect(screen.getAllByText('Press Send to share them.')).toHaveLength(1);
      // A server path alone is not a file waiting to be sent: no hint.
      rerenderWith({
        draft: { body: '', attachments: [], references: [{ id: 'r', label: 'Run' }] },
      });
      expect(screen.queryByText(/Press Send/)).toBeNull();
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
      // Pause shows once the chip has been up a second (Q3-16).
      const pause = await screen.findByRole(
        'button',
        { name: 'Pause counts.csv' },
        { timeout: 2000 }
      );
      await userEvent.setup().click(pause);
      expect(mocks.pauseTransfer).toHaveBeenCalledWith('transfer-1');
    });

    it('shows a new upload as “Uploading…” with no 0% and no Pause for its first second (Q3-16)', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      try {
        mocks.listTransfers.mockResolvedValue([
          {
            id: 'transfer-2',
            request_id: 'request-2',
            connection_id: 'connection-1',
            channel_id: 'channel-1',
            direction: 'upload',
            name: 'gina-assay.csv',
            size: 100,
            sha256: '',
            offset: 0,
            blob_id: null,
            state: 'uploading',
            error: null,
          },
        ]);
        renderComposer();
        expect(await screen.findByText(filesCopy.uploading)).toBeInTheDocument();
        expect(screen.queryByText('0%')).toBeNull();
        expect(screen.queryByRole('button', { name: 'Pause gina-assay.csv' })).toBeNull();
        await act(async () => {
          vi.advanceTimersByTime(1000);
        });
        expect(screen.getByText('0%')).toBeInTheDocument();
        const pause = screen.getByRole('button', { name: 'Pause gina-assay.csv' });
        // Named for the file; its tooltip says what it does, since a glyph alone read as "Paused?".
        fireEvent.focus(pause);
        expect(filesCopy.pauseUpload).toBe('Pause upload');
      } finally {
        vi.useRealTimers();
      }
    });
  });

  describe('a file already in the channel (Q3-13)', () => {
    /** Mounts the composer beside a stand-in card that registers a file already in #general. */
    function Registered({
      blobId,
      name,
      sha256,
      postedAt,
    }: {
      blobId: string;
      name: string;
      sha256: string;
      postedAt: number;
    }) {
      const index = useAttachmentIndex();
      // A card registers what it learned once it has loaded, as AttachmentCard does.
      useEffect(
        () => index?.register(blobId, { name, sha256, complete: true, postedAt }),
        [index, blobId, name, sha256, postedAt]
      );
      return null;
    }

    function renderWithShared(overrides: Partial<CrewController>, shared: ReactNode) {
      return render(
        <CrewTestProvider controller={crewTestController(overrides)}>
          <AttachmentIndexProvider>
            {shared}
            <Composer />
          </AttachmentIndexProvider>
        </CrewTestProvider>
      );
    }

    const today = new Date();
    const at = new Date(today.getFullYear(), today.getMonth(), today.getDate(), 18, 54).getTime();

    it('notes a same-named file under its chip, without blocking the send', () => {
      renderWithShared(
        {
          draft: {
            body: '',
            attachments: [{ id: 'blob-2', name: 'gina-assay.csv' }],
            references: [],
          },
        },
        <Registered blobId="blob-1" name="gina-assay.csv" sha256={'a'.repeat(64)} postedAt={at} />
      );
      expect(
        screen.getByText(
          'gina-assay.csv is already in #general (shared 6:54 PM). Remove this one if it’s the same file.'
        )
      ).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Send message' })).toBeEnabled();
    });

    it('notes a file with the same contents under another name, once its checksum is known', async () => {
      mocks.listTransfers.mockResolvedValue([
        {
          id: 'transfer-7',
          request_id: 'request-7',
          connection_id: 'connection-1',
          channel_id: 'channel-1',
          direction: 'upload',
          name: 'copy of assay.csv',
          size: 100,
          sha256: 'a'.repeat(64),
          offset: 100,
          blob_id: 'blob-2',
          state: 'completed',
          error: null,
        },
      ]);
      renderWithShared(
        {
          draft: {
            body: '',
            attachments: [{ id: 'blob-2', name: 'copy of assay.csv' }],
            references: [],
          },
        },
        <Registered blobId="blob-1" name="gina-assay.csv" sha256={'a'.repeat(64)} postedAt={at} />
      );
      expect(
        await screen.findByText(/^copy of assay\.csv is already in #general \(shared 6:54 PM\)/)
      ).toBeInTheDocument();
    });

    it('says nothing for a different file, or with no channel view around it', () => {
      renderWithShared(
        { draft: { body: '', attachments: [{ id: 'blob-2', name: 'other.csv' }], references: [] } },
        <Registered blobId="blob-1" name="gina-assay.csv" sha256={'a'.repeat(64)} postedAt={at} />
      );
      expect(screen.queryByText(/is already in #general/)).toBeNull();
      cleanup();
      renderComposer({
        draft: {
          body: '',
          attachments: [{ id: 'blob-2', name: 'gina-assay.csv' }],
          references: [],
        },
      });
      expect(screen.queryByText(/is already in #general/)).toBeNull();
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

    it('still shows its own failure when there is no channel to write in', () => {
      const { rerenderWith } = renderComposer({ channel: null, channelId: '' });
      expect(screen.queryByRole('textbox')).toBeNull();
      expect(screen.queryByText(composerCopy.verifying)).toBeNull();
      rerenderWith({ error: { message: 'send failed', source: 'composer' } });
      expect(screen.getAllByText('send failed')).toHaveLength(1);
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

  describe('an upload failure never lingers over the layout note', () => {
    const originalElectron = (window as { electron?: unknown }).electron;
    const note = <p>Connect this chat to #general?</p>;
    const screenshot = () => new File(['x'], 'image.png', { type: 'image/png' });

    beforeEach(() => {
      // A desktop surface: the secure picker exists, and a pasted screenshot has no file.
      (window as { electron?: unknown }).electron = {
        crewSelectTransferFile: vi.fn(),
        getPathForFile: () => '',
      };
    });
    afterEach(() => {
      (window as { electron?: unknown }).electron = originalElectron;
    });

    async function pasteScreenshot() {
      // In `act`, so the dismiss control's tooltip settles inside the test that mounted it.
      await act(async () => {
        fireEvent.paste(screen.getByLabelText('Message #general'), {
          clipboardData: { files: [screenshot()] },
        });
      });
      const alert = screen.getByRole('alert');
      expect(alert).toHaveTextContent(filesCopy.notSaved);
      expect(screen.getAllByRole('alert')).toHaveLength(1);
      // It answers what the person just did, so it has the slot for now.
      expect(screen.queryByText('Connect this chat to #general?')).toBeNull();
      expect(mocks.beginTransfer).not.toHaveBeenCalled();
    }

    const expectNoteBack = () => {
      expect(screen.queryByText(filesCopy.notSaved)).toBeNull();
      expect(screen.queryByRole('alert')).toBeNull();
      expect(screen.getByText('Connect this chat to #general?')).toBeInTheDocument();
    };

    it('clears a pasted screenshot’s refusal when the person types', async () => {
      render(<StatefulComposer note={note} />);
      await pasteScreenshot();
      await userEvent.setup().type(screen.getByLabelText('Message #general'), 'h');
      expect(screen.getByLabelText('Message #general')).toHaveValue('h');
      expectNoteBack();
    });

    it('clears it when the person sends, even a send that leaves the draft in place', async () => {
      const send = vi.fn(async () => undefined);
      render(<StatefulComposer note={note} initialBody="hello" overrides={{ send }} />);
      await pasteScreenshot();
      fireEvent.keyDown(screen.getByLabelText('Message #general'), {
        key: 'Enter',
        code: 'Enter',
        keyCode: 13,
      });
      expect(send).toHaveBeenCalledTimes(1);
      expect(screen.getByLabelText('Message #general')).toHaveValue('hello');
      expectNoteBack();
    });

    it('clears it after a successful send that empties the draft', async () => {
      render(<StatefulComposer note={note} initialBody="hello" />);
      await pasteScreenshot();
      await userEvent.setup().click(screen.getByRole('button', { name: 'Send message' }));
      expect(screen.getByLabelText('Message #general')).toHaveValue('');
      expectNoteBack();
    });

    it('has a dismiss control that returns the note and puts focus back in the text', async () => {
      render(<StatefulComposer note={note} />);
      await pasteScreenshot();
      const alert = screen.getByRole('alert');
      // One word, as every dismiss control says (Q2-61).
      expect(composerCopy.dismissUploadError).toBe('Dismiss');
      expect(within(alert).getByRole('button', { name: 'Dismiss' })).toBeInTheDocument();
      await userEvent
        .setup()
        .click(within(alert).getByRole('button', { name: composerCopy.dismissUploadError }));
      expectNoteBack();
      expect(screen.getByLabelText('Message #general')).toHaveFocus();
    });

    it('clears it on a reset that cleared the protected state', async () => {
      let listener: SurfaceResetListener | undefined;
      render(
        <StatefulComposer
          note={note}
          overrides={{
            subscribeSurfaceReset: (next) => {
              listener = next;
              return () => undefined;
            },
          }}
        />
      );
      await pasteScreenshot();
      await act(async () => listener?.('protected-cleared'));
      expectNoteBack();
    });

    it('drops the privacy refusal once the observer verifies the mode it asked for', async () => {
      const unverified: Partial<CrewController> = {
        observedPrivacy: {
          connectionId: 'another-connection',
          mode: 'private',
          institutionId: null,
          policyEpoch: 1,
        },
      };
      const { rerender } = render(<StatefulComposer note={note} overrides={unverified} />);
      const user = userEvent.setup();
      await user.click(screen.getByRole('button', { name: 'Attach' }));
      await user.click(await screen.findByRole('menuitem', { name: 'Upload a file…' }));
      expect(await screen.findByRole('alert')).toHaveTextContent(filesCopy.privacyPending);
      expect(screen.queryByText('Connect this chat to #general?')).toBeNull();
      expect(mocks.beginTransfer).not.toHaveBeenCalled();

      // The observer verifies this connection's privacy: the advice is now wrong.
      rerender(<StatefulComposer note={note} />);
      expect(screen.queryByText(filesCopy.privacyPending)).toBeNull();
      expect(screen.queryByRole('alert')).toBeNull();
      expect(screen.getByText('Connect this chat to #general?')).toBeInTheDocument();
    });

    it('still gives way to a send failure', async () => {
      const { rerender } = render(<StatefulComposer note={note} />);
      await pasteScreenshot();
      rerender(
        <StatefulComposer
          note={note}
          overrides={{ error: { message: 'send failed', source: 'composer' } }}
        />
      );
      expect(screen.getAllByRole('alert')).toHaveLength(1);
      expect(screen.getByRole('alert')).toHaveTextContent('Couldn’t send. send failed');
    });
  });

  describe('a file while a file window is already open (T-26)', () => {
    const originalElectron = (window as { electron?: unknown }).electron;
    beforeEach(() => {
      // A desktop surface: the secure picker exists, and a copied file has a path on disk.
      (window as { electron?: unknown }).electron = {
        crewSelectTransferFile: vi.fn(),
        getPathForFile: () => '/Users/dave/counts.csv',
      };
    });
    afterEach(() => {
      (window as { electron?: unknown }).electron = originalElectron;
    });

    it('says to finish the open window instead of doing nothing, and lets the note go with it', async () => {
      let finish!: () => void;
      mocks.beginTransfer.mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = () => resolve(null);
          })
      );
      renderComposer();
      const user = userEvent.setup();
      await user.click(screen.getByRole('button', { name: 'Attach' }));
      await user.click(await screen.findByRole('menuitem', { name: 'Upload a file…' }));
      expect(mocks.beginTransfer).toHaveBeenCalledTimes(1);

      await act(async () => {
        fireEvent.paste(screen.getByLabelText('Message #general'), {
          clipboardData: { files: [new File(['x'], 'counts.csv')], getData: () => '' },
        });
      });
      expect(screen.getByRole('status')).toHaveTextContent(filesCopy.finishChoosing);
      expect(filesCopy.finishChoosing).toBe(
        'Finish choosing a file in the open file window first.'
      );
      // Nothing new opened: the open window is still the one.
      expect(mocks.beginTransfer).toHaveBeenCalledTimes(1);

      await act(async () => finish());
      expect(screen.queryByText(filesCopy.finishChoosing)).toBeNull();
    });
  });

  describe('a dropped or pasted file asks once, in the native dialog (D-DROP, Q2-16)', () => {
    const originalElectron = (window as { electron?: unknown }).electron;
    let share: ReturnType<typeof vi.fn>;
    let picker: ReturnType<typeof vi.fn>;
    const counts = () => new File(['x,y'], 'counts.csv', { type: 'text/csv' });
    const named = (): Partial<CrewController> => {
      const snapshot = crewTestController().snapshot!;
      return { snapshot: { ...snapshot, workspace: { ...snapshot.workspace, name: 'lab' } } };
    };

    beforeEach(() => {
      share = vi.fn();
      picker = vi.fn();
      (window as { electron?: unknown }).electron = {
        crewShareDroppedFile: share,
        crewSelectTransferFile: picker,
        getPathForFile: () => '/Users/dave/Downloads/counts.csv',
      };
    });
    afterEach(() => {
      (window as { electron?: unknown }).electron = originalElectron;
    });

    async function paste(file = counts()) {
      await act(async () => {
        fireEvent.paste(screen.getByLabelText('Message #general'), {
          clipboardData: { files: [file], getData: () => '' },
        });
      });
      return file;
    }

    it('hands the preload the File itself and the channel, then uploads with the capability it gets', async () => {
      let answer!: (result: unknown) => void;
      share.mockImplementation(
        () =>
          new Promise((resolve) => {
            answer = resolve;
          })
      );
      mocks.beginTransfer.mockResolvedValue({ id: 'transfer-3' });
      renderComposer(named());
      const file = await paste();

      // While the dialog is up, the note says where to answer; nothing has been shared yet. It
      // names no file: a dropped shortcut resolves to its target in the dialog (Q3-25).
      expect(screen.getByRole('status')).toHaveTextContent(filesCopy.confirmShare);
      expect(filesCopy.confirmShare).toBe('To share it, choose Share in the dialog.');
      expect(screen.getByRole('status')).not.toHaveTextContent('counts.csv');
      expect(share).toHaveBeenCalledTimes(1);
      const [sent, destination] = share.mock.calls[0];
      expect(sent).toBe(file);
      // Never a path: the preload resolves the File, and main shows the path in its own dialog.
      expect(destination).toEqual({
        expectedMode: 'private',
        connectionId: 'connection-1',
        channelId: 'channel-1',
        channelName: 'general',
        workspaceName: 'lab',
      });
      expect(JSON.stringify(destination)).not.toContain('/Users/');
      expect(mocks.beginTransfer).not.toHaveBeenCalled();
      expect(picker).not.toHaveBeenCalled();

      await act(async () =>
        answer({ outcome: 'shared', capability_id: 'cap-9', name: 'counts.csv', size: 3 })
      );
      expect(mocks.beginTransfer).toHaveBeenCalledWith(
        {
          expected_mode: 'private',
          connection_id: 'connection-1',
          channel_id: 'channel-1',
          direction: 'upload',
        },
        { capability_id: 'cap-9', name: 'counts.csv', size: 3 }
      );
      expect(picker).not.toHaveBeenCalled();
      expect(screen.queryByText(filesCopy.confirmShare)).toBeNull();
      expect(screen.queryByRole('alert')).toBeNull();
    });

    it('does nothing more when the person chooses Cancel', async () => {
      share.mockResolvedValue({ outcome: 'cancelled' });
      renderComposer(named());
      await paste();
      expect(mocks.beginTransfer).not.toHaveBeenCalled();
      expect(screen.queryByRole('alert')).toBeNull();
      expect(screen.queryByRole('status')).toBeNull();
    });

    it('shows the main process’s refusal as the one upload error', async () => {
      share.mockResolvedValue({
        outcome: 'refused',
        message: '"results" is a folder. Crew shares one file at a time.',
      });
      renderComposer(named());
      await paste();
      expect(screen.getByRole('alert')).toHaveTextContent(
        '"results" is a folder. Crew shares one file at a time.'
      );
      expect(mocks.beginTransfer).not.toHaveBeenCalled();
    });

    it('treats an answer it does not understand as a failed upload, never as a share', async () => {
      share.mockResolvedValue({ outcome: 'shared', name: 'counts.csv' });
      renderComposer(named());
      await paste();
      expect(screen.getByRole('alert')).toHaveTextContent(filesCopy.uploadFailed);
      expect(mocks.beginTransfer).not.toHaveBeenCalled();
    });

    it('opens one confirmation at a time, and says to finish the open one', async () => {
      let answer!: (result: unknown) => void;
      share.mockImplementation(
        () =>
          new Promise((resolve) => {
            answer = resolve;
          })
      );
      renderComposer(named());
      await paste();
      await paste(new File(['z'], 'other.csv'));
      expect(share).toHaveBeenCalledTimes(1);
      expect(screen.getByRole('status')).toHaveTextContent(filesCopy.finishConfirming);
      await act(async () => answer({ outcome: 'cancelled' }));
      expect(screen.queryByText(filesCopy.finishConfirming)).toBeNull();
    });

    it('asks nothing until the workspace’s privacy is verified', async () => {
      renderComposer({
        ...named(),
        observedPrivacy: {
          connectionId: 'another-connection',
          mode: 'private',
          institutionId: null,
          policyEpoch: 1,
        },
      });
      await paste();
      expect(share).not.toHaveBeenCalled();
      expect(screen.getByRole('alert')).toHaveTextContent(filesCopy.privacyPending);
    });

    it('refuses a folder and a file over the limit before any dialog opens', async () => {
      renderComposer(named());
      const huge = counts();
      Object.defineProperty(huge, 'size', { value: 2 * 1024 * 1024 * 1024 });
      await paste(huge);
      expect(screen.getByRole('alert')).toHaveTextContent(filesCopy.tooLarge('counts.csv'));
      expect(share).not.toHaveBeenCalled();
    });

    it('without the confirmation, opens the picker and says what to pick there, and where it is', async () => {
      (window as { electron?: unknown }).electron = {
        crewSelectTransferFile: picker,
        getPathForFile: () => '/Users/dave/Downloads/counts.csv',
      };
      let finish!: () => void;
      mocks.beginTransfer.mockImplementation(
        () =>
          new Promise((resolve) => {
            finish = () => resolve(null);
          })
      );
      renderComposer(named());
      await paste();
      const note = screen.getByRole('status');
      expect(note).toHaveTextContent(
        'A file window opened. Select counts.csv (it’s in Downloads) there and choose Open to share it.'
      );
      // The folder is display text: the picker is asked exactly as the Attach menu asks it.
      expect(mocks.beginTransfer).toHaveBeenCalledWith({
        expected_mode: 'private',
        connection_id: 'connection-1',
        channel_id: 'channel-1',
        direction: 'upload',
      });
      await act(async () => finish());
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

    it('says whether the agent pane is open, and which element it opens (Q3-23)', async () => {
      const closePane = vi.fn();
      const openPane = vi.fn();
      const controller = crewTestController({ openPane, closePane });
      const view = render(
        <CrewTestProvider controller={controller}>
          <Composer agentPaneId="agent-pane" />
        </CrewTestProvider>
      );
      const ask = screen.getByRole('button', { name: 'Ask my agent' });
      expect(ask).toHaveAttribute('aria-expanded', 'false');
      expect(ask).toHaveAttribute('aria-controls', 'agent-pane');

      view.rerender(
        <CrewTestProvider
          controller={crewTestController({
            openPane,
            closePane,
            ui: { dialog: null, pane: { mode: 'agent' } },
          })}
        >
          <Composer agentPaneId="agent-pane" />
        </CrewTestProvider>
      );
      expect(ask).toHaveAttribute('aria-expanded', 'true');
      // Expanded, it collapses: the same press closes what it opened.
      await userEvent.setup().click(ask);
      expect(closePane).toHaveBeenCalledTimes(1);
      expect(openPane).not.toHaveBeenCalled();

      // Another pane (the channel details) is not the agent pane.
      view.rerender(
        <CrewTestProvider
          controller={crewTestController({
            openPane,
            closePane,
            ui: { dialog: null, pane: { mode: 'details', tab: 'about' } },
          })}
        >
          <Composer agentPaneId="agent-pane" />
        </CrewTestProvider>
      );
      expect(ask).toHaveAttribute('aria-expanded', 'false');
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
      // Words only, like every other Crew menu (Q2-61).
      expect(menu.querySelector('[role="menuitem"] svg')).toBeNull();
      // Beside the paperclip, not over the note above the card (Q2-61).
      expect(menu).toHaveAttribute('data-side', 'right');
      expect(menu).toHaveAttribute('data-align', 'end');
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
