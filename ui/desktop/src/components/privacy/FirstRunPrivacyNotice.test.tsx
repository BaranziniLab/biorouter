import { cleanup, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

/**
 * Task 38 (issue #56 §15.5) — the day-one notice.
 *
 * ⚠ **What this file can and cannot pin, stated up front because a previous
 * version of it got this wrong in its test names.** The visible-vs-raw property
 * — that the notice quotes what History shows, not the raw `sessions` count —
 * is enforced in SQL, by `list_sessions_by_types`' `INNER JOIN messages`, so
 * `GET /sessions` never hands the renderer an invisible row at all. There is
 * therefore no fixture this layer can build that would separate the two
 * populations: `totalVisible` is `sessions.length`, and it would be
 * `sessions.length` under a broken daemon too. That property is genuinely tested
 * one layer down, in `session_manager.rs`'s
 * `the_notice_quotes_the_history_visible_count_not_the_raw_one`, whose fixture
 * carries a message-less row and whose assertion moves when the `EXISTS` is
 * removed.
 *
 * What IS testable here is everything about the population the renderer is
 * handed: the bucketing rule, the fail-closed tier read, the provider
 * breakdown, which question the notice asks the daemon, and — the part review
 * found missing — whether the notice is ever shown at all.
 */
import {
  computeNoticeCounts,
  FirstRunPrivacyNotice,
  providerLabel,
  shouldShowFirstRunNotice,
  UNKNOWN_PROVIDER,
} from './FirstRunPrivacyNotice';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
}));

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<Record<string, unknown>>()),
  listSessions: mocks.listSessions,
}));

/** A row shaped the way `GET /sessions` serves one. */
function row(
  id: string,
  tier: 'private' | 'public' | undefined,
  provider: string | null,
  reason?: string
): Session {
  return {
    id,
    name: id,
    description: '',
    working_dir: '/tmp',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    extension_data: {} as Session['extension_data'],
    message_count: 1,
    privacy_tier: tier,
    privacy_reason: reason ?? null,
    provider_name: provider,
  } as Session;
}

/** `GET /sessions` as the generated client returns it. */
function served(sessions: Session[]) {
  return { data: { sessions } };
}

/**
 * The same eight-session shape the Rust test uses, minus the one row the daemon
 * would never send — so the two implementations are checked against one fixture.
 */
const FIXTURE: Session[] = [
  row('s1', 'private', 'versa_azure', 'backfill:versa_azure'),
  row('s2', 'private', 'versa_azure', 'backfill:versa_azure'),
  row('s4', 'public', 'anthropic'),
  row('s5', 'public', 'anthropic'),
  row('s6', 'public', 'anthropic'),
  row('s7', 'public', null),
  row('s8', 'public', null),
];

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('computeNoticeCounts', () => {
  // ⚠ Named for what it does. It was called "quotes the History-visible count,
  // not the raw one", which is a property this layer cannot see — see the file
  // header. It buckets the rows the daemon sent; the Rust test owns the rest.
  it('partitions the served rows into private, public-named and provider-unknown', () => {
    const counts = computeNoticeCounts(FIXTURE);
    expect(counts.privateVisible).toBe(2);
    expect(counts.publicNamedVisible).toBe(3);
    expect(counts.unknownProviderVisible).toBe(2);
    expect(counts.totalVisible).toBe(7);
    // The three buckets partition the denominator, on every input.
    expect(counts.privateVisible + counts.publicNamedVisible + counts.unknownProviderVisible).toBe(
      counts.totalVisible
    );
  });

  it('groups the private rows by the provider the migration read', () => {
    const counts = computeNoticeCounts([
      ...FIXTURE,
      row('s9', 'private', 'ollama', 'backfill:ollama'),
      row('s10', 'private', 'llamacpp', 'backfill:llamacpp'),
      row('s11', 'private', 'llamacpp', 'backfill:llamacpp'),
    ]);
    expect(counts.privateByProvider).toEqual([
      { provider: 'llamacpp', count: 2 },
      { provider: 'versa_azure', count: 2 },
      { provider: 'ollama', count: 1 },
    ]);
  });

  it('reads a private row whose provenance is not a backfill from its bound provider', () => {
    // A chat raised by Gate C carries `mcp:<extension>`, not `backfill:*`. It is
    // still private and still belongs in the breakdown.
    const counts = computeNoticeCounts([row('s1', 'private', 'versa_azure', 'mcp:ucsfomopagent')]);
    expect(counts.privateByProvider).toEqual([{ provider: 'versa_azure', count: 1 }]);
  });

  it('files a private row with no provenance at all under the unknown bucket', () => {
    const counts = computeNoticeCounts([row('s1', 'private', null)]);
    expect(counts.privateByProvider).toEqual([{ provider: UNKNOWN_PROVIDER, count: 1 }]);
    expect(providerLabel(UNKNOWN_PROVIDER)).toBe('Provider not recorded');
  });

  it('fails closed on a tier it cannot read', () => {
    // A daemon too old to send the column, or a projection that dropped it.
    // `PrivacyBadge` will paint these rows Private; a notice that counted them
    // public would quote a friendlier number than the badges beside it.
    const counts = computeNoticeCounts([row('s1', undefined, 'anthropic')]);
    expect(counts.privateVisible).toBe(1);
    expect(counts.publicNamedVisible).toBe(0);
  });

  it('passes an unrecognised provider id through rather than dropping it', () => {
    expect(providerLabel('some_future_provider')).toBe('some_future_provider');
  });
});

describe('shouldShowFirstRunNotice', () => {
  it('is false when nothing visible came out private', () => {
    expect(shouldShowFirstRunNotice(computeNoticeCounts([row('s1', 'public', 'anthropic')]))).toBe(
      false
    );
  });

  it('is true as soon as one visible chat was marked by the migration', () => {
    expect(shouldShowFirstRunNotice(computeNoticeCounts(FIXTURE))).toBe(true);
  });

  it('stays false on a machine the migration never marked, however private it gets', () => {
    // The whole notice is about a thing the upgrade did. A fresh install
    // accumulates private chats one turn at a time — `turn:*`, never
    // `backfill:*` — and firing on those would ambush that user weeks later
    // with a modal describing a migration that never touched their database.
    const grownOrganically = [
      row('s1', 'private', 'ollama', 'turn:ollama'),
      row('s2', 'private', 'ucsfomopagent', 'mcp:ucsfomopagent'),
      row('s3', 'private', 'versa_azure', 'diverged:s1'),
    ];
    const counts = computeNoticeCounts(grownOrganically);
    expect(counts.privateVisible).toBe(3);
    expect(counts.backfilledVisible).toBe(0);
    expect(shouldShowFirstRunNotice(counts)).toBe(false);
  });
});

describe('FirstRunPrivacyNotice', () => {
  it('computes its numbers from the user own chat list', async () => {
    mocks.listSessions.mockResolvedValue(served(FIXTURE));
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} />);

    const headline = await screen.findByTestId('notice-headline');
    expect(headline.textContent).toContain('2');
    expect(headline.textContent).toContain('7');
    // The chats it will not vouch for are stated, not buried.
    expect(screen.getByTestId('notice-unknown').textContent).toContain('2');
    // ...and it asked the daemon for the History population directly, rather
    // than through `sessionListCache`. That module's `includeSubagents` argument
    // is part of its cache IDENTITY: passing `false` here reset a user who had
    // subagents shown, and passing nothing would have counted them when the
    // cache happened to hold `true`. Either way the notice's denominator stops
    // matching the window it claims to describe.
    expect(mocks.listSessions).toHaveBeenCalledWith(
      expect.objectContaining({ query: { include_subagents: false } })
    );
  });

  it('lists the private chats by the model that marked them', async () => {
    mocks.listSessions.mockResolvedValue(served(FIXTURE));
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} />);

    const list = await screen.findByTestId('notice-by-provider');
    expect(within(list).getByText(/Versa \(Azure\): 2/)).toBeTruthy();
  });

  it('says the tier came from the last model used, and how to repair it', () => {
    render(
      <FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={computeNoticeCounts(FIXTURE)} />
    );
    const caveat = screen.getByTestId('notice-last-model-caveat').textContent ?? '';
    expect(caveat).toContain('last using');
    expect(caveat).toContain('switch it to a private model');
  });

  it('names the knowledge-base control, not only the exposure', () => {
    render(
      <FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={computeNoticeCounts(FIXTURE)} />
    );
    const kb = screen.getByTestId('notice-knowledge-bases').textContent ?? '';
    expect(kb).toContain('start public');
    expect(kb).toContain('mark it private yourself');
  });

  it('admits it could not count rather than reporting zero', async () => {
    mocks.listSessions.mockRejectedValue(new Error('daemon down'));
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} />);

    await screen.findByTestId('notice-count-error');
    expect(screen.queryByTestId('notice-headline')).toBeNull();
  });

  it('does not read the chat list until it is opened', () => {
    render(<FirstRunPrivacyNotice open={false} onDismiss={vi.fn()} />);
    expect(mocks.listSessions).not.toHaveBeenCalled();
  });

  it('acknowledging closes it', async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    mocks.listSessions.mockResolvedValue(served(FIXTURE));
    render(<FirstRunPrivacyNotice open onDismiss={onDismiss} />);

    await user.click(await screen.findByTestId('notice-acknowledge'));
    await waitFor(() => expect(onDismiss).toHaveBeenCalledTimes(1));
  });

  // §13.5's day-one extension disclosure. Open question 13 resolves the naming
  // question to "Task 38's notice copy" and says to hard-code the machine's own
  // expectation into the fixture — hence `medcp` by name below.
  it('names the enabled public extension that is wired to clinical data', () => {
    render(
      <FirstRunPrivacyNotice
        open
        onDismiss={vi.fn()}
        counts={computeNoticeCounts(FIXTURE)}
        publicClinicalExtensions={['medcp']}
      />
    );
    const paragraph = screen.getByTestId('notice-public-clinical-extensions').textContent ?? '';
    expect(paragraph).toContain('medcp');
    // The disclosure is about REACHABILITY, not about a change. Saying "now
    // marked" of an extension nothing happened to would be false, and would send
    // the user looking for a setting that moved.
    expect(paragraph).toMatch(/commercial models hosted outside your institution/i);
    expect(paragraph).toMatch(/nothing about it has changed/i);
  });

  // FR-S-14, class only (privacy UI, ruling 15): the one disclosure no refusal will ever teach is
  // in the default ink, not the muted one the less critical knowledge-base line wears, and the
  // counts are tabular figures.
  it('sets the clinical-extension disclosure in default ink and the counts as tabular figures', () => {
    render(
      <FirstRunPrivacyNotice
        open
        onDismiss={vi.fn()}
        counts={computeNoticeCounts(FIXTURE)}
        publicClinicalExtensions={['medcp']}
      />
    );
    expect(screen.getByTestId('notice-public-clinical-extensions')).not.toHaveClass(
      'text-text-muted'
    );
    expect(screen.getByTestId('first-run-privacy-notice')).toHaveClass('tabular-nums');
  });

  it('says nothing about extensions when there are none to name', () => {
    render(
      <FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={computeNoticeCounts(FIXTURE)} />
    );
    expect(screen.queryByTestId('notice-public-clinical-extensions')).toBeNull();
  });
});

/**
 * R-34, first run round 1 (FR-R1-14). On an upgraded install the notice is the first screen, and it
 * was the one raw `DialogContent` on the path (560px, centred, no header rule, `text-sm`) while every
 * other first-run dialog was the shared shell. jsdom lays nothing out, so this pins only the anatomy
 * the shell sets; `firstRunPrivacyNotice.browser.test.tsx` measures where it sits.
 */
describe('FirstRunPrivacyNotice is the shared dialog', () => {
  const counts = computeNoticeCounts(FIXTURE);
  const surface = () => document.querySelector('[data-slot="dialog-content"]') as HTMLElement;

  it('is a ModalShell at size md, pinned to the top, with the header rule', () => {
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={counts} />);
    expect(surface().getAttribute('data-size')).toBe('md');
    expect(surface().getAttribute('data-anchor')).toBe('top');
    expect(surface().className).toContain('sm:max-w-[480px]');
    expect(surface().className).not.toContain('sm:max-w-[560px]');
    const header = surface().firstElementChild as HTMLElement;
    expect(header.className).toContain('border-b');
    expect(within(header).getByRole('heading').textContent).toBe(
      'Some of your chats are now marked private'
    );
  });

  it('sets its prose in the body role, not the legacy text-sm', () => {
    render(
      <FirstRunPrivacyNotice
        open
        onDismiss={vi.fn()}
        counts={counts}
        publicClinicalExtensions={['medcp']}
      />
    );
    expect(surface().querySelector('.text-sm, .text-xs, .text-base')).toBeNull();
    const body = screen.getByTestId('first-run-privacy-notice');
    expect(body.className).toContain('text-body');
    expect(screen.getByTestId('notice-headline').className).toContain('text-body');
  });

  it('keeps the count sentence as the dialog description', async () => {
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={counts} />);
    const dialog = screen.getByRole('dialog');
    expect(dialog.getAttribute('aria-describedby')).toBe(screen.getByTestId('notice-headline').id);
    expect(dialog).toHaveAccessibleDescription(
      screen.getByTestId('notice-headline').textContent ?? ''
    );
  });

  it('describes the dialog by the counting and the could-not-count lines too', async () => {
    let release: (value: unknown) => void = () => {};
    mocks.listSessions.mockReturnValue(new Promise((resolve) => (release = resolve)));
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} />);
    const dialog = screen.getByRole('dialog');
    expect(dialog.getAttribute('aria-describedby')).toBe(screen.getByTestId('notice-counting').id);
    release(served(FIXTURE));
    await screen.findByTestId('notice-headline');
    expect(dialog.getAttribute('aria-describedby')).toBe(screen.getByTestId('notice-headline').id);
  });

  it('puts the acknowledgement in the footer, and the shell offers the one close control', () => {
    render(<FirstRunPrivacyNotice open onDismiss={vi.fn()} counts={counts} />);
    const footer = screen.getByTestId('notice-acknowledge').parentElement as HTMLElement;
    expect(footer.className).toContain('justify-end');
    expect(surface().contains(footer)).toBe(true);
    expect(screen.getAllByRole('button', { name: 'Close' })).toHaveLength(1);
  });

  it('is dismissed by Escape, as it was', async () => {
    const user = userEvent.setup();
    const onDismiss = vi.fn();
    render(<FirstRunPrivacyNotice open onDismiss={onDismiss} counts={counts} />);
    await user.keyboard('{Escape}');
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
