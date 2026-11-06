import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeAll, beforeEach, afterAll, describe, expect, it, vi } from 'vitest';
import { ComposerFooter, type ComposerFooterProps } from './ComposerFooter';
import { FOLDER_ACCESS_COPY, FOOTER_COPY } from './copy';
import { listFolderAccess, revokeFolderAccess } from '../../api';

vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'gpt-5.6-sol', currentProvider: 'versa_azure' }),
}));
// The folder chip's card reads a chat's grants only when it opens; only the
// unknown-folder test below opens it, and the guard below says so.
vi.mock('../../api', () => ({
  listFolderAccess: vi.fn(async () => ({ data: [] })),
  revokeFolderAccess: vi.fn(async () => ({ data: { revoked: true } })),
  updateWorkingDir: vi.fn(async () => ({ data: {} })),
}));
vi.mock('../../utils/pricing', () => ({
  fetchModelPricing: vi.fn(async () => ({
    input_token_cost: 0.00001,
    output_token_cost: 0.00003,
    currency: '$',
  })),
}));

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
afterAll(() => vi.unstubAllGlobals());
// Calls only, not the mocks' answers: "nothing is read" below means in that test.
beforeEach(() => vi.clearAllMocks());

const BASE: ComposerFooterProps = {
  sessionId: 'chat-1',
  workingDir: '/Users/wgu/Desktop',
  workingDirLocked: true,
  onWorkingDirChange: vi.fn(),
  totalTokens: 92_000,
  tokenLimit: 128_000,
  isTokenLimitLoaded: true,
  onCompact: vi.fn(),
  modelCostRows: [
    {
      provider: 'versa_azure',
      model: 'gpt-5.6-sol',
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      totalTokens: 1200,
      turns: 2,
      totalCost: 0.42,
      costIsPartial: false,
    },
  ],
};

/**
 * A folder the folder card writes (`FooterCardPath`), found by the absolute path it stands for: its
 * names sit in separate `<bdi>`s, so no text query finds the path whole.
 */
const byPath = (container: Element | Document, absolute: string) =>
  [...container.querySelectorAll<HTMLElement>('[data-path]')].find(
    (node) => node.dataset.path === absolute
  ) ?? null;

/** Let the cost tracker's pricing fetch land, so its state update is inside act. */
async function settle() {
  await act(async () => {});
}

describe('ComposerFooter (spec 3.7)', () => {
  it('names the folder on the left and reads context and cost on the right in a chat', async () => {
    render(<ComposerFooter {...BASE} />);
    await settle();
    const footer = screen.getByTestId('composer-footer');
    expect(footer).toHaveClass('br-footline');
    expect(screen.getByText('Desktop')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: FOOTER_COPY.contextTooltip(28, '92k', '128k') })
    ).toBeInTheDocument();
    expect(screen.getByText(FOOTER_COPY.contextLeft(28))).toBeInTheDocument();
    expect(screen.getByTestId('chat-cost')).toHaveTextContent('$0.42');
  });

  // Folder access moved into the folder chip's card: the line has no button of
  // its own for it, and nothing is read until the card opens.
  it('has no separate folder access button; the folder chip carries it', async () => {
    render(<ComposerFooter {...BASE} />);
    await settle();
    expect(screen.queryByRole('button', { name: 'Folder access' })).toBeNull();
    expect(screen.queryByText('Folder access')).toBeNull();
    const chip = screen.getByTestId('dir-switcher-locked');
    expect(chip).toHaveAccessibleName(FOOTER_COPY.folderLabel('/Users/wgu/Desktop'));
    expect(chip).toHaveAttribute('aria-haspopup', 'dialog');
    expect(listFolderAccess).not.toHaveBeenCalled();
  });

  it('hands the composer card to the line’s cards, which hang above it', async () => {
    // jsdom places nothing, so the geometry is the browser test's
    // (`footerCard.browser.test.tsx`); this pins only that the footer's
    // `FooterCardGroup` passes the composer through rather than dropping it.
    const composer = { current: document.createElement('div') };
    const spy = vi.spyOn(composer.current, 'getBoundingClientRect');
    render(<ComposerFooter {...BASE} composerRef={composer} />);
    await settle();
    fireEvent.click(screen.getByTestId('dir-switcher-locked'), { detail: 1 });
    await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
    await waitFor(() => expect(spy).toHaveBeenCalled());
  });

  it('shows only the folder on Home: no ring, no "100%", no "$0.00"', async () => {
    render(<ComposerFooter {...BASE} sessionId={null} workingDirLocked={false} />);
    await settle();
    expect(screen.getByText('Desktop')).toBeInTheDocument();
    expect(screen.queryByTestId('context-window-indicator')).toBeNull();
    expect(screen.queryByTestId('chat-cost')).toBeNull();
    expect(screen.queryByText('100%')).toBeNull();
    expect(screen.queryByText('$0.00')).toBeNull();
  });

  // V2-R7-3: an existing chat whose folder is not known yet names no folder,
  // rather than the app's default it does not use. The rest of the line is the
  // chat's own and stays, and the folder's arrival changes nothing beside it.
  //
  // ⚠ The chip itself stays, nameless: its card is the only place the chat's
  // other folders can be revoked, and that must not wait on the folder's row
  // (which a failed read leaves unknown for the life of the chat).
  describe('a chat whose folder is not known yet (workingDir null)', () => {
    it('names no folder but keeps the folder chip and both sides of the line', async () => {
      render(<ComposerFooter {...BASE} workingDir={null} />);
      await settle();
      const footer = screen.getByTestId('composer-footer');
      const sides = footer.querySelectorAll('.br-footline__side');
      expect(sides).toHaveLength(2);
      // The chip on the left names nothing: the glyph alone, no folder named
      // by a default the chat does not use.
      const chip = within(sides[0] as HTMLElement).getByTestId('dir-switcher-unknown');
      expect(chip).toHaveAccessibleName(FOOTER_COPY.folderUnknownLabel);
      expect(chip).toHaveAttribute('aria-haspopup', 'dialog');
      expect(chip).toHaveTextContent('');
      expect(footer.querySelector('.br-footline__name')).toBeNull();
      expect(screen.queryByTestId('dir-switcher-locked')).toBeNull();
      expect(screen.queryByText('Desktop')).toBeNull();
      // The ring and the cost are about the chat, not its folder.
      expect(screen.getByTestId('context-window-indicator')).toBeInTheDocument();
      expect(screen.getByTestId('chat-cost')).toHaveTextContent('$0.42');
    });

    it('still lists and revokes the chat’s other folders from that chip', async () => {
      const grant = { id: 'grant-9', folder: '/synthetic/shared', access: 'read-only' };
      vi.mocked(listFolderAccess)
        .mockResolvedValueOnce({ data: [{ ...grant, descendants: true }] } as never)
        .mockResolvedValueOnce({ data: [] } as never);
      render(<ComposerFooter {...BASE} workingDir={null} />);
      await settle();
      fireEvent.click(screen.getByTestId('dir-switcher-unknown'), { detail: 1 });
      const card = await screen.findByRole('dialog', { name: FOLDER_ACCESS_COPY.cardLabel });
      expect(card).toHaveTextContent(FOLDER_ACCESS_COPY.workingFolderUnknown);
      await waitFor(() => expect(byPath(card, grant.folder)).toHaveTextContent(grant.folder));
      fireEvent.click(
        within(card).getByRole('button', { name: FOLDER_ACCESS_COPY.revokeLabel(grant.folder) })
      );
      await waitFor(() =>
        expect(revokeFolderAccess).toHaveBeenCalledWith(
          expect.objectContaining({ body: { sessionId: 'chat-1', grantId: 'grant-9' } })
        )
      );
      expect(await within(card).findByText(FOLDER_ACCESS_COPY.none)).toBeInTheDocument();
    });

    it('draws the folder when it arrives and leaves the ring and cost nodes alone', async () => {
      const { rerender } = render(<ComposerFooter {...BASE} workingDir={null} />);
      await settle();
      const ring = screen.getByTestId('context-window-indicator');
      const arc = screen.getByTestId('context-ring-arc');
      const cost = screen.getByTestId('chat-cost');

      rerender(<ComposerFooter {...BASE} workingDir="/Users/wgu/Desktop" />);
      await settle();
      expect(screen.getByText('Desktop')).toBeInTheDocument();
      // Same elements, not remounted ones: a remount would re-arm the arc's
      // first-reading rule and the cost's pricing fetch beside a folder name.
      expect(screen.getByTestId('context-window-indicator')).toBe(ring);
      expect(screen.getByTestId('context-ring-arc')).toBe(arc);
      expect(screen.getByTestId('chat-cost')).toBe(cost);
    });

    it('still draws nothing but the folder for a composer that has no chat', async () => {
      render(<ComposerFooter {...BASE} sessionId={null} workingDirLocked={false} />);
      await settle();
      const footer = screen.getByTestId('composer-footer');
      expect(footer.querySelectorAll('.br-footline__side')).toHaveLength(1);
      expect(screen.getByText('Desktop')).toBeInTheDocument();
    });
  });

  it('hides a $0.00 cost inside a chat', async () => {
    render(
      <ComposerFooter {...BASE} modelCostRows={[{ ...BASE.modelCostRows![0], totalCost: 0 }]} />
    );
    await settle();
    expect(screen.queryByTestId('chat-cost')).toBeNull();
    expect(screen.queryByText('$0.00')).toBeNull();
  });

  it('has no monospace text anywhere on the line', async () => {
    const { container } = render(<ComposerFooter {...BASE} />);
    await settle();
    expect(container.querySelector('.font-mono')).toBeNull();
  });

  // V2-R3-5: the ring knows its chat, so another chat's reading under the same
  // footer lands at rest instead of sweeping from this chat's arc.
  it('hands the ring its chat, so a change of chat does not sweep the arc', async () => {
    const { rerender } = render(<ComposerFooter {...BASE} totalTokens={10_000} />);
    await settle();
    rerender(<ComposerFooter {...BASE} sessionId="chat-2" totalTokens={92_000} />);
    expect(screen.getByTestId('context-ring-arc')).toHaveAttribute('data-motion-still');
  });

  // V2-R4-1, round 5: the ring hears which model the window belongs to, so the
  // new-chat model's window never reads as a loading chat's.
  it('hands the ring the model, so a window for another model draws nothing', async () => {
    const { rerender } = render(
      <ComposerFooter {...BASE} boundModel={null} tokenLimitModel="versa_azure/gpt-5.5" />
    );
    await settle();
    expect(screen.queryByTestId('context-window-indicator')).toBeNull();
    rerender(
      <ComposerFooter
        {...BASE}
        boundModel="anthropic/claude-opus-4-5"
        tokenLimitModel="versa_azure/gpt-5.5"
      />
    );
    expect(screen.queryByTestId('context-window-indicator')).toBeNull();
    rerender(
      <ComposerFooter
        {...BASE}
        boundModel="anthropic/claude-opus-4-5"
        tokenLimitModel="anthropic/claude-opus-4-5"
      />
    );
    expect(
      screen.getByRole('button', { name: FOOTER_COPY.contextTooltip(28, '92k', '128k') })
    ).toBeInTheDocument();
  });
});
