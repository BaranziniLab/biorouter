import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { stripHiddenCharacters } from '../../../utils/untrustedText';
import { ComposerChips } from '../composer/ComposerChips';
import type { CrewTransfer } from '../crewTransfers';
import { AttachmentCard, type CrewBlob } from './AttachmentCard';
import { clearBlobCache } from './blobMetadataCache';
import { saveNameFor, visibleFileText } from './fileName';
import { ServerPathRow } from './ServerPathRow';
import { TransferRow } from './TransferRow';
import { UploadChip } from './UploadChip';

/**
 * RENDERER-1: a file name, a server path's label and the path itself are chosen by another member,
 * and the broker only refuses control characters (Cc) in them. A format character such as U+202E
 * RIGHT-TO-LEFT OVERRIDE got through and was drawn as written, so `q3_{U+202E}fdp.terminal` read
 * `q3_lanimret.pdf` on every card, tooltip and control name, and was the Save dialog's default
 * name. Every such string is now shown with its hidden characters made visible (U+FFFD), and the
 * default save name leaves them out.
 */

const mocks = vi.hoisted(() => ({
  crewRequest: vi.fn(),
  listTransfers: vi.fn(),
  beginTransfer: vi.fn(),
  pauseTransfer: vi.fn(),
  resumeTransfer: vi.fn(),
  forgetTransfer: vi.fn(),
  previewAttachment: vi.fn(),
}));

vi.mock('../crewApi', () => ({ crewRequest: mocks.crewRequest }));
vi.mock('../crewTransfers', () => ({
  listTransfers: mocks.listTransfers,
  beginTransfer: mocks.beginTransfer,
  pauseTransfer: mocks.pauseTransfer,
  resumeTransfer: mocks.resumeTransfer,
  forgetTransfer: mocks.forgetTransfer,
  previewAttachment: mocks.previewAttachment,
}));

const RLO = '\u202E';
/** Renders as `q3_lanimret.pdf` when the override is left in. */
const SPOOFED = `q3_${RLO}fdp.terminal`;
const SHOWN = 'q3_�fdp.terminal';
const SAVED = 'q3_fdp.terminal';

/** Whether `text` holds a character that draws nothing of its own: never drawn as written. */
const hasHidden = (text: string) =>
  stripHiddenCharacters(text) !== text || /[\p{Zl}\p{Zp}]/u.test(text);

function expectNothingHidden(root: HTMLElement) {
  expect(hasHidden(root.textContent ?? '')).toBe(false);
  for (const element of root.querySelectorAll('*')) {
    for (const attribute of ['aria-label', 'title', 'alt']) {
      expect(hasHidden(element.getAttribute(attribute) ?? '')).toBe(false);
    }
  }
}

const transfer = (overrides: Partial<CrewTransfer> = {}): CrewTransfer => ({
  id: 'transfer-1',
  request_id: 'request-1',
  connection_id: 'connection-1',
  channel_id: 'channel-1',
  direction: 'download',
  name: SPOOFED,
  size: 1000,
  sha256: 'f'.repeat(64),
  offset: 500,
  blob_id: 'blob-1',
  state: 'needs_file_selection',
  error: null,
  ...overrides,
});

beforeEach(() => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  mocks.listTransfers.mockResolvedValue([]);
  clearBlobCache();
});

describe('a file name another member chose', () => {
  it('shows an attachment’s hidden characters, in its text and every control’s name', async () => {
    const blob: CrewBlob = {
      id: 'blob-1',
      channel_id: 'channel-1',
      name: SPOOFED,
      size: 1000,
      sha256: 'f'.repeat(64),
      complete: true,
      media_type: 'application/octet-stream',
    };
    mocks.crewRequest.mockResolvedValue(blob);
    const { container } = render(<AttachmentCard connectionId="connection-1" blobId="blob-1" />);
    expect(await screen.findByText(SHOWN)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: `Save ${SHOWN}` })).toBeInTheDocument();
    expectNothingHidden(container);
  });

  it('offers the Save dialog the name without its hidden characters', async () => {
    mocks.crewRequest.mockResolvedValue({
      id: 'blob-1',
      channel_id: 'channel-1',
      name: SPOOFED,
      size: 1000,
      sha256: 'f'.repeat(64),
      complete: true,
      media_type: 'application/octet-stream',
    } satisfies CrewBlob);
    mocks.beginTransfer.mockResolvedValue(null);
    render(<AttachmentCard connectionId="connection-1" blobId="blob-1" />);
    await screen.findByText(SHOWN);
    await userEvent.setup().click(screen.getByRole('button', { name: `Save ${SHOWN}` }));
    expect(mocks.beginTransfer).toHaveBeenCalledWith(
      expect.objectContaining({ direction: 'download', blob_id: 'blob-1', suggestedName: SAVED })
    );
  });

  it('never offers the Save dialog a name that is only hidden characters', async () => {
    mocks.crewRequest.mockResolvedValue({
      id: 'blob-1',
      channel_id: 'channel-1',
      name: `${RLO}\u200B`,
      size: 1000,
      sha256: 'f'.repeat(64),
      complete: true,
      media_type: 'application/octet-stream',
    } satisfies CrewBlob);
    mocks.beginTransfer.mockResolvedValue(null);
    render(<AttachmentCard connectionId="connection-1" blobId="blob-1" />);
    const save = await screen.findByRole('button', { name: 'Save ��' });
    await userEvent.setup().click(save);
    expect(mocks.beginTransfer.mock.calls[0]?.[0]?.suggestedName).toBeUndefined();
  });

  it('shows a transfer row’s and an upload chip’s name with its hidden characters visible', () => {
    const actions = { onPause: vi.fn(), onResume: vi.fn(), onRemove: vi.fn() };
    const row = render(
      <ul>
        <TransferRow transfer={transfer()} {...actions} />
      </ul>
    );
    expect(within(row.container).getByText(SHOWN)).toBeInTheDocument();
    expectNothingHidden(row.container);
    row.unmount();

    const chip = render(
      <UploadChip
        transfer={transfer({ direction: 'upload' })}
        onPause={vi.fn()}
        onResume={vi.fn()}
      />
    );
    expect(within(chip.container).getByText(SHOWN)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: `Resume ${SHOWN}` })).toBeInTheDocument();
    expectNothingHidden(chip.container);
  });

  it('keeps the exact name for Resume, which must reopen the same destination', async () => {
    const actions = { onPause: vi.fn(), onResume: vi.fn(), onRemove: vi.fn() };
    render(
      <ul>
        <TransferRow transfer={transfer()} {...actions} />
      </ul>
    );
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: `More actions for ${SHOWN}` }));
    await user.click(await screen.findByRole('menuitem', { name: 'Resume…' }));
    expect(actions.onResume).toHaveBeenCalledWith(expect.objectContaining({ name: SPOOFED }));
  });

  it('shows a composer chip’s file name and reference label with hidden characters visible', () => {
    const { container } = render(
      <ComposerChips
        attachments={[{ id: 'blob-1', name: SPOOFED }]}
        references={[{ id: 'ref-1', label: `results${RLO}vsc.sh` }]}
        onRemoveAttachment={vi.fn()}
        onRemoveReference={vi.fn()}
        onPauseUpload={vi.fn()}
        onResumeUpload={vi.fn()}
      />
    );
    expect(screen.getByText(SHOWN)).toBeInTheDocument();
    expect(screen.getByText('results�vsc.sh')).toBeInTheDocument();
    expectNothingHidden(container);
  });
});

describe('the two rules (visibleFileText, saveNameFor)', () => {
  const samples = [
    SPOOFED,
    'report\u200B.pdf',
    'two\nlines.txt',
    'line\u2028separator.txt',
    'isolate\u2066d\u2069.csv',
    'tag\u{E0041}.txt',
    'wide   gap.csv',
    'ordinary name (1).csv',
    'naïve résumé.pdf',
    '\u{1F469}\u200D\u{1F4BB} notes.md',
  ];

  it('shows each hidden character as U+FFFD, and nothing else changes', () => {
    expect(visibleFileText(SPOOFED)).toBe(SHOWN);
    expect(visibleFileText('ordinary name (1).csv')).toBe('ordinary name (1).csv');
    expect(visibleFileText('naïve résumé.pdf')).toBe('naïve résumé.pdf');
    for (const sample of samples) expect(hasHidden(visibleFileText(sample))).toBe(false);
    expect(visibleFileText(undefined)).toBe('');
    expect(visibleFileText(42)).toBe('');
  });

  it('reads a name as the native share dialog does (utils/crewSharePath.ts)', async () => {
    // The dialog's rule is main-process code the renderer cannot import, so the two are held
    // together here: the same file must read the same in the dialog and on its card.
    const { visibleText } = await import('../../../utils/crewSharePath');
    for (const sample of samples) expect(visibleFileText(sample)).toBe(visibleText(sample));
  });

  it('leaves hidden characters out of a save name, and has no name when nothing is left', () => {
    expect(saveNameFor(SPOOFED)).toBe(SAVED);
    expect(saveNameFor(' two\nlines.txt ')).toBe('twolines.txt');
    expect(saveNameFor('counts.csv')).toBe('counts.csv');
    for (const sample of samples) expect(hasHidden(saveNameFor(sample) ?? '')).toBe(false);
    expect(saveNameFor('\u202E\u200B')).toBeUndefined();
    expect(saveNameFor('\u202E..')).toBeUndefined();
    expect(saveNameFor(null)).toBeUndefined();
  });
});

describe('a server path another member shared', () => {
  it('shows its label and path with hidden characters visible, and copies the path exactly', async () => {
    const path = `/lab/shared/q3_${RLO}fdp.terminal`;
    mocks.crewRequest.mockResolvedValue({ path, label: `Q3 ${RLO}stluser`, verified: false });
    const writeText = vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
    const { container } = render(<ServerPathRow connectionId="connection-1" referenceId="ref-1" />);
    expect(await screen.findByText('Q3 �stluser')).toBeInTheDocument();
    expectNothingHidden(container);
    await userEvent.setup().click(screen.getByRole('button', { name: /^Copy server path/ }));
    expect(writeText).toHaveBeenCalledWith(path);
  });

  it('falls back to the path, made visible, when the label is empty', async () => {
    mocks.crewRequest.mockResolvedValue({
      path: `/lab/${RLO}fdp.sh`,
      label: '',
      verified: false,
    });
    const { container } = render(<ServerPathRow connectionId="connection-1" referenceId="ref-1" />);
    expect((await screen.findAllByText('/lab/�fdp.sh')).length).toBeGreaterThan(0);
    expectNothingHidden(container);
  });
});
