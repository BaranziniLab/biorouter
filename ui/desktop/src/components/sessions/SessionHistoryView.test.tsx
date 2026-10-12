import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionHistoryView from './SessionHistoryView';
import { MAKE_CHAT_PUBLIC } from './copy';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({ declassifySession: vi.fn(), getSession: vi.fn() }));

vi.mock('../../api', () => ({
  declassifySession: mocks.declassifySession,
  getSession: mocks.getSession,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

// See SessionItem.test.tsx: this file deliberately never names the badge
// component, because Task 27's gate greps src/components for that name and
// expects an exact file list.

vi.mock('../ProgressiveMessageList', () => ({
  default: () => <div data-testid="messages" />,
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

function session(over: Partial<Session> = {}): Session {
  return {
    id: 'session-1',
    name: 'Cohort query',
    created_at: '2026-07-14T12:00:00Z',
    updated_at: '2026-07-14T12:00:00Z',
    working_dir: '/tmp',
    message_count: 0,
    extension_data: {},
    conversation: [],
    ...over,
  } as Session;
}

function renderView(over: Partial<Session> = {}, showActionButtons = false) {
  return render(
    <MemoryRouter>
      <SessionHistoryView
        session={session(over)}
        isLoading={false}
        error={null}
        onBack={vi.fn()}
        onRetry={vi.fn()}
        showActionButtons={showActionButtons}
      />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  // The shape the generated client resolves a 200 with. The dialog reads the
  // status off the Response, so a bare `{}` — no Response at all — is a failed
  // request, exactly as it is at runtime.
  mocks.declassifySession.mockResolvedValue({
    data: { sessionId: 'x', privacyTier: 'public' },
    response: { status: 200 },
  });
});

describe('SessionHistoryView — the privacy marker', () => {
  it('marks a private session in the header', () => {
    renderView({ privacy_tier: 'private' });
    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');
  });

  it('names the tier in words on this roomy header, including for a public session', () => {
    renderView({ privacy_tier: 'public' });
    const badge = screen.getByTestId('privacy-badge');
    expect(badge).toHaveAttribute('data-privacy', 'public');
    expect(badge).toHaveTextContent('Public');
  });

  it('says nothing at all when the session carries no tier', () => {
    renderView();
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });
});

// Issue #56 §12.1's second entry point. It shares `DeclassifySessionDialog` with
// History's row menu so the two cannot come to ask for different confirmations.
describe('SessionHistoryView — declassification', () => {
  it('offers Make public only on a private session', () => {
    const view = renderView({ privacy_tier: 'public' }, true);
    expect(screen.queryByRole('button', { name: MAKE_CHAT_PUBLIC })).toBeNull();
    view.unmount();

    renderView({ privacy_tier: 'private' }, true);
    expect(screen.getByRole('button', { name: MAKE_CHAT_PUBLIC })).toBeInTheDocument();
  });

  it('clears the header badge once the chat is public, without waiting for a refetch', async () => {
    // `mcp:*`, so §12.4 grades this onto the typed confirmation and the request
    // goes out on confirm. The `turn:*` path is the same code with a 5-second
    // hold in front of it, and its window is exercised where it can be shortened
    // (`DeclassifySessionDialog.test.tsx`), not against a real five seconds here.
    renderView(
      { privacy_tier: 'private', privacy_reason: 'mcp:ucsfomopagent', id: '20260714_120000' },
      true
    );
    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');

    fireEvent.click(screen.getByRole('button', { name: MAKE_CHAT_PUBLIC }));

    // Scoped to the dialog, whose confirm button is the one named "Make public".
    const dialog = await screen.findByRole('dialog');
    fireEvent.change(within(dialog).getByRole('textbox'), { target: { value: '120000' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Make public' }));

    await waitFor(() => expect(mocks.declassifySession).toHaveBeenCalled());
    await waitFor(() =>
      expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'public')
    );
  });

  // Item 11 of the 1.90.4 hold (2026-09-13): a declassification made in ANOTHER
  // window reaches this page too. The page reads its `session` prop once, and
  // the daemon no longer re-sorts the chat to say something happened, so the
  // badge follows the row announcement — and states the daemon's read of it.
  it('follows a declassification made in another window', async () => {
    mocks.getSession.mockResolvedValue({
      data: {
        id: '20260714_130000',
        privacy_tier: 'public',
        privacy_reason: 'declassified_by_user',
      },
    });
    renderView({
      privacy_tier: 'private',
      privacy_reason: 'turn:versa_azure',
      id: '20260714_130000',
    });
    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');

    const otherWindow = new BroadcastChannel('biorouter:session-row');
    try {
      // A different chat first: this page must not move for it.
      otherWindow.postMessage({ sessionId: 'someone-else' });
      otherWindow.postMessage({ sessionId: '20260714_130000' });
      await waitFor(() =>
        expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'public')
      );
    } finally {
      otherWindow.close();
    }
    expect(mocks.getSession).toHaveBeenCalledWith(
      expect.objectContaining({ path: { session_id: '20260714_130000' } })
    );
  });
});

/**
 * The saved transcript sits on the CHAT measure, and on ONE measure. It used to
 * draw its conversation in a `max-w-4xl` box nested inside the page's reading
 * column — the 896px "replay column" the design of record §4.4 retires — so a
 * saved chat was rendered at a width the live chat never uses, and the inner
 * ceiling won whatever the outer one said.
 *
 * ⚠ jsdom sees the ATTRIBUTE and the class STRING, never a width: there is no
 * layout engine and Tailwind never runs, so `max-w-measure-chat` computes to
 * nothing here. The 760px column was measured in a browser. The complementary
 * source rule — no `<ReadableContent` without a `size`, and no second
 * `max-w-*` anywhere in the file — lives in `styles/measures.test.ts`.
 */
describe('SessionHistoryView sits on one chat measure', () => {
  it('renders a single reading column, at the chat size', () => {
    const { container } = renderView();

    const columns = [...container.querySelectorAll('.biorouter-readable-content')];
    expect(columns).toHaveLength(1);
    expect((columns[0] as HTMLElement).dataset.size).toBe('chat');
  });

  it('nests no second measure inside it', () => {
    const { container } = renderView({
      conversation: [
        {
          id: 'm1',
          role: 'user',
          created: 1757200000,
          metadata: { provenance: null },
          content: [{ type: 'text', text: 'hello' }],
        },
      ],
      message_count: 1,
    } as Partial<Session>);

    // Every descendant, not just the transcript wrapper: the fork this replaces
    // was one `<div>` deep inside a conditional branch, which is exactly where
    // a replacement would go too.
    for (const element of container.querySelectorAll<HTMLElement>('*')) {
      expect(element.className.toString()).not.toMatch(/\bmax-w-(?:3xl|4xl|5xl|6xl|7xl)\b/);
    }
  });
});

/**
 * The saved transcript takes the 44px band (spec 3.4): Back, the chat's title,
 * the privacy marker beside it from the first frame, and the actions as round
 * icons. It used to be a 24px `<h1>` with a two-line mono metadata block.
 */
describe('SessionHistoryView band', () => {
  it('puts Back, the title, the marker and the actions on one band', () => {
    renderView({ privacy_tier: 'private', message_count: 3 }, true);

    const heading = screen.getByRole('heading', { level: 1, name: 'Cohort query' });
    expect(heading).toHaveClass('text-label');
    const band = heading.closest('[data-band]') as HTMLElement;
    expect(band).not.toBeNull();
    expect(within(band).getByRole('button', { name: 'Back' })).toBeInTheDocument();
    expect(within(band).getByTestId('privacy-badge')).toBeInTheDocument();
    expect(within(band).getByRole('button', { name: 'Share' })).toBeInTheDocument();
    expect(within(band).getByRole('button', { name: 'Resume' })).toBeInTheDocument();
    expect(within(band).getByText(/3 messages/)).toBeInTheDocument();
  });

  it('shows the marker while the chat is still loading', () => {
    render(
      <MemoryRouter>
        <SessionHistoryView
          session={session({ privacy_tier: 'private' })}
          isLoading={true}
          error={null}
          onBack={vi.fn()}
          onRetry={vi.fn()}
        />
      </MemoryRouter>
    );
    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'private');
  });

  it('keeps Share reachable while it is unavailable, so its reason can be read', () => {
    localStorage.removeItem('session_sharing_config');
    renderView({}, true);
    const share = screen.getByRole('button', { name: 'Share' });
    expect(share).toHaveAttribute('aria-disabled', 'true');
    expect(share).not.toBeDisabled();
  });

  it('sits in the preview split as its header area, beside the panel', () => {
    const { container } = renderView({}, true);
    const header = container.querySelector('[data-preview-area="header"]');
    expect(header?.querySelector('[data-band]')).not.toBeNull();
    // The band is the grid's header area; the column under it is the transcript's.
    const transcript = container.querySelector('[data-preview-area="transcript"]');
    expect(transcript?.querySelector('.biorouter-readable-content')).not.toBeNull();
    expect(header?.compareDocumentPosition(transcript as Node)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );
  });
});
