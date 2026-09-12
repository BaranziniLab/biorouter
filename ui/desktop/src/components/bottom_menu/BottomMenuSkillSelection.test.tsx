import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BottomMenuSkillSelection } from './BottomMenuSkillSelection';
import type { CatalogBundle, CatalogSkill, CatalogView } from '../../api';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';

const mocks = vi.hoisted(() => ({
  overrides: new Map<string, boolean>(),
  saveSkillOverrides: vi.fn(async () => undefined),
  loadSkillOverrides: vi.fn(async () => true),
  skillCatalogHandler: vi.fn(),
  refreshSkillCatalog: vi.fn(),
  setSessionSkills: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
}));

vi.mock('../../store/skillOverrides', () => ({
  loadSkillOverrides: mocks.loadSkillOverrides,
  saveSkillOverrides: mocks.saveSkillOverrides,
  setSkillOverride: (name: string, enabled: boolean) => mocks.overrides.set(name, enabled),
  isSkillEnabled: (name: string) => mocks.overrides.get(name) ?? true,
  getSkillOverrides: () => mocks.overrides,
}));

vi.mock('../../api', () => ({
  skillCatalogHandler: (...args: unknown[]) => mocks.skillCatalogHandler(...args),
  refreshSkillCatalog: (...args: unknown[]) => mocks.refreshSkillCatalog(...args),
  setSessionSkills: (...args: unknown[]) => mocks.setSessionSkills(...args),
}));

vi.mock('../../toasts', () => ({
  toastService: { success: mocks.toastSuccess, error: mocks.toastError },
}));

function skill(name: string, extras: Partial<CatalogSkill> = {}): CatalogSkill {
  return {
    name,
    description: `${name} description`,
    slug: name,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    bundle: null,
    builtin: false,
    state: {
      machineEnabled: true,
      session: 'default',
      sessionViaBundle: false,
      hiddenContext: false,
      effective: true,
    },
    ...extras,
  };
}

function bundle(
  name: string,
  members: string[],
  extras: Partial<CatalogBundle> = {}
): CatalogBundle {
  return {
    name,
    displayName: name,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    skills: members,
    package: null,
    builtin: false,
    state: {
      machineEnabled: true,
      session: 'default',
      sessionViaBundle: false,
      hiddenContext: false,
      effective: true,
    },
    ...extras,
  };
}

function view(overrides: Partial<CatalogView> = {}): CatalogView {
  return {
    generation: 1,
    roots: [],
    skills: [skill('example-skill')],
    bundles: [],
    ...overrides,
  };
}

function serve(next: CatalogView) {
  mocks.skillCatalogHandler.mockResolvedValue({ data: next });
  mocks.refreshSkillCatalog.mockResolvedValue({ data: next });
}

async function openMenu() {
  fireEvent.pointerDown(screen.getByLabelText(/Manage skills/), { button: 0, ctrlKey: false });
  return screen.findAllByRole('menuitemcheckbox');
}

describe('BottomMenuSkillSelection', () => {
  beforeEach(() => {
    mocks.overrides.clear();
    vi.clearAllMocks();
    serve(view());
  });

  // ---------------------------------------------------------------- #113 (a)

  /**
   * The defect this component existed to have: the session branch wrote React
   * state, raised a green toast, and called nothing. Asserting the REQUEST is
   * what makes that irreproducible — a re-introduced local-only path would
   * still flip the switch and still pass any test that only reads the switch.
   */
  it('sends a per-chat toggle to the daemon and scopes it to the session', async () => {
    mocks.setSessionSkills.mockResolvedValue({
      data: {
        catalog: view({
          skills: [
            skill('example-skill', {
              state: {
                machineEnabled: true,
                session: 'removed',
                sessionViaBundle: false,
                hiddenContext: false,
                effective: false,
              },
            }),
          ],
        }),
        sessionAdd: [],
        sessionRemove: ['example-skill'],
      },
    });

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.setSessionSkills).toHaveBeenCalledTimes(1));
    expect(mocks.setSessionSkills.mock.calls[0][0].body).toEqual({
      sessionId: '20260824_1',
      add: [],
      remove: ['example-skill'],
    });
    // ...and it must not have gone anywhere near the machine-wide file.
    expect(mocks.saveSkillOverrides).not.toHaveBeenCalled();
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
  });

  it('raises a success toast only after the backend confirms, and rolls back when it refuses', async () => {
    mocks.setSessionSkills.mockRejectedValue(new Error('session not found'));

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.toastError.mock.calls[0][0].msg).toContain('session not found');
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  /**
   * Two fast clicks must land in the order they were made. Without the queue
   * in `useSkillCatalog`, the second request can resolve first and the switch
   * settles on the OLDER of the two answers — a toggle that looks like it
   * ignored the last click.
   */
  it('serialises concurrent toggles so the last click wins', async () => {
    const order: string[] = [];
    let releaseFirst: (() => void) | null = null;
    mocks.setSessionSkills.mockImplementation(async (options: { body: { remove: string[] } }) => {
      const disabling = options.body.remove.length > 0;
      order.push(disabling ? 'disable' : 'enable');
      if (disabling) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return {
        data: {
          catalog: view({
            skills: [
              skill('example-skill', {
                state: {
                  machineEnabled: true,
                  session: disabling ? 'removed' : 'added',
                  sessionViaBundle: false,
                  hiddenContext: false,
                  effective: !disabling,
                },
              }),
            ],
          }),
          sessionAdd: disabling ? [] : ['example-skill'],
          sessionRemove: disabling ? ['example-skill'] : [],
        },
      };
    });

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();

    fireEvent.click(toggle); // disable — parked
    await waitFor(() => expect(order).toEqual(['disable']));
    fireEvent.click(toggle); // enable — must wait its turn
    expect(order).toEqual(['disable']);

    releaseFirst!();
    await waitFor(() => expect(order).toEqual(['disable', 'enable']));
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  /**
   * The rollback must undo only the toggle that failed.
   *
   * ⚠ **Both clicks happen in ONE render**, with no `await` between them, and
   * that is the whole test. With the previous catalog read from the render
   * closure rather than from what the hook last committed, both callbacks
   * capture the same starting catalog — so a refusal on the second restored the
   * state from before the FIRST, silently undoing on screen a change the daemon
   * had already accepted. Awaiting between the clicks lets React re-render and
   * hands the second callback a fresh closure, which hides the defect
   * completely: the first draft of this test passed against the broken code.
   */
  it('a refused second toggle does not undo a first one the daemon accepted', async () => {
    serve(view({ skills: [skill('alpha'), skill('beta')] }));
    const disabledAlpha = view({
      skills: [
        skill('alpha', {
          state: {
            machineEnabled: true,
            session: 'removed',
            sessionViaBundle: false,
            hiddenContext: false,
            effective: false,
          },
        }),
        skill('beta'),
      ],
    });
    mocks.setSessionSkills.mockImplementation(async (o: { body: { remove: string[] } }) => {
      if (o.body.remove.includes('beta')) throw new Error('refused');
      return { data: { catalog: disabledAlpha, sessionAdd: [], sessionRemove: ['alpha'] } };
    });

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [alpha, beta] = await openMenu();

    fireEvent.click(alpha);
    fireEvent.click(beta);

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(beta).toHaveAttribute('aria-checked', 'true'));
    expect(alpha).toHaveAttribute('aria-checked', 'false');
  });

  it('says "for this chat" only when there is a chat', async () => {
    mocks.setSessionSkills.mockResolvedValue({
      data: { catalog: view(), sessionAdd: [], sessionRemove: ['example-skill'] },
    });
    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    fireEvent.click(toggle);
    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalledTimes(1));
    expect(mocks.toastSuccess.mock.calls[0][0].msg).toContain('for this chat');
  });

  // ---------------------------------------------------------------- #113 (b)

  /**
   * BiorOffice and MarkItDown ship skills inside their extension directory.
   * The backend has always loaded them; this picker's own three-root scan never
   * saw them, so they were active for the model and had no row here.
   */
  it('lists a skill bundled inside an installed extension, and names the extension', async () => {
    serve(
      view({
        skills: [
          skill('example-skill'),
          skill('word', {
            sourceRoot: '/extensions/BiorOffice/skills',
            source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
          }),
        ],
      })
    );
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();
    expect(await screen.findByText('word')).toBeInTheDocument();
    expect(screen.getByText('BiorOffice')).toBeInTheDocument();
  });

  it('asks the daemon to rescan when the menu opens, so an install made elsewhere shows up', async () => {
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();
    await waitFor(() => expect(mocks.refreshSkillCatalog).toHaveBeenCalled());
  });

  it('refetches and refreshes this chat when the catalog changes', async () => {
    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    await openMenu();
    await waitFor(() => expect(mocks.refreshSkillCatalog).toHaveBeenCalled());
    const callsBeforeChange = mocks.refreshSkillCatalog.mock.calls.length;

    mocks.refreshSkillCatalog.mockResolvedValue({
      data: view({ skills: [skill('example-skill'), skill('extension-arrived')] }),
    });
    await act(async () => {
      window.dispatchEvent(new CustomEvent(CATALOG_CHANGED_EVENT, { detail: { revision: 1 } }));
    });

    await waitFor(() =>
      expect(mocks.refreshSkillCatalog.mock.calls.length).toBeGreaterThan(callsBeforeChange)
    );
    expect(mocks.refreshSkillCatalog).toHaveBeenLastCalledWith({
      query: { session_id: '20260824_1' },
      throwOnError: true,
    });
    expect(await screen.findByText('extension-arrived')).toBeInTheDocument();
    expect(screen.getByLabelText('Manage skills (2 enabled)')).toBeInTheDocument();
  });

  // ------------------------------------------------------------------ bundles

  it('toggles a bundle by its own name rather than expanding its members', async () => {
    serve(view({ skills: [], bundles: [bundle('hyperframes', ['hyperframes', 'media-use'])] }));
    mocks.setSessionSkills.mockResolvedValue({
      data: {
        catalog: view({
          skills: [],
          bundles: [bundle('hyperframes', ['hyperframes', 'media-use'])],
        }),
        sessionAdd: [],
        sessionRemove: ['hyperframes'],
      },
    });

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.setSessionSkills).toHaveBeenCalledTimes(1));
    expect(mocks.setSessionSkills.mock.calls[0][0].body.remove).toEqual(['hyperframes']);
  });

  it('shows a package bundle as one row with its member count and entry point', async () => {
    serve(
      view({
        skills: [],
        bundles: [
          bundle('hyperframes', ['hyperframes', 'media-use'], {
            displayName: 'HyperFrames',
            package: {
              id: 'hyperframes',
              displayName: 'HyperFrames',
              version: '0.8.12',
              entryPoint: 'hyperframes',
              sourceUrl: null,
              sourceRef: null,
              resolvedCommit: null,
              installer: null,
              installedAt: null,
              groups: {},
            },
          }),
        ],
      })
    );
    render(<BottomMenuSkillSelection sessionId={null} />);
    const items = await openMenu();
    expect(items).toHaveLength(1);
    expect(screen.getByText('HyperFrames')).toBeInTheDocument();
    expect(screen.getByText('2 skills')).toBeInTheDocument();
    expect(screen.getByText(/entry point: hyperframes/)).toBeInTheDocument();
  });

  // -------------------------------------------------------------- hub (machine)

  it('applies rapid hub choices in order and leaves the final choice enabled', async () => {
    render(<BottomMenuSkillSelection sessionId={null} />);
    const [toggle] = await openMenu();
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    serve(
      view({
        skills: [
          skill('example-skill', {
            state: {
              machineEnabled: false,
              session: 'default',
              sessionViaBundle: false,
              hiddenContext: false,
              effective: false,
            },
          }),
        ],
      })
    );
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));

    serve(view());
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    expect(mocks.overrides.get('example-skill')).toBe(true);
  });

  it('restores the persisted state when the machine-wide write fails', async () => {
    render(<BottomMenuSkillSelection sessionId={null} />);
    const [toggle] = await openMenu();

    mocks.saveSkillOverrides.mockRejectedValueOnce(new Error('disk full'));
    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    // The in-memory store is re-read, or the next save would persist the edit
    // whose write just failed.
    expect(mocks.loadSkillOverrides).toHaveBeenCalled();
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  /**
   * ⚠ **The chip counts ROWS, and a Context bundle is a row.**
   * `activeCount` filters nothing itself — every Context exclusion happens in
   * `useSkillCatalog`, and until the knowledge skills became a bundle every one
   * of those tested a skill's own `name`. A bundle carries none of its members'
   * names, so promoting them put one row back in the picker and +1 on the chip,
   * for a thing the user manages in Settings.
   *
   * The count must therefore be `1` here, not `2`: `example-skill` alone.
   */
  it('leaves a Context bundle out of the picker and out of the chip count', async () => {
    serve(
      view({
        skills: [
          skill('example-skill'),
          skill('knowledge-lint', { bundle: 'knowledge-bases', builtin: true }),
          skill('update-soul', { bundle: 'knowledge-bases', builtin: true }),
        ],
        bundles: [bundle('knowledge-bases', ['knowledge-lint', 'update-soul'], { builtin: true })],
      })
    );
    render(<BottomMenuSkillSelection sessionId={null} />);

    expect(await screen.findByLabelText('Manage skills (1 enabled)')).toBeInTheDocument();
    const rows = await openMenu();
    expect(rows).toHaveLength(1);
    expect(screen.queryByText('knowledge-bases')).not.toBeInTheDocument();
    expect(screen.queryByText('knowledge-lint')).not.toBeInTheDocument();
  });

  /**
   * An installed bundle is still a row and still counted, so the assertion
   * above is about Contexts and not about bundles generally.
   */
  it('still counts an installed bundle as one row', async () => {
    serve(
      view({
        skills: [skill('example-skill'), skill('media-use', { bundle: 'hyperframes' })],
        bundles: [bundle('hyperframes', ['media-use'])],
      })
    );
    render(<BottomMenuSkillSelection sessionId={null} />);

    expect(await screen.findByLabelText('Manage skills (2 enabled)')).toBeInTheDocument();
  });

  it('reports a catalog it could not read instead of claiming there are no skills', async () => {
    mocks.skillCatalogHandler.mockRejectedValue(new Error('daemon is down'));
    mocks.refreshSkillCatalog.mockRejectedValue(new Error('daemon is down'));
    render(<BottomMenuSkillSelection sessionId={null} />);
    fireEvent.pointerDown(screen.getByLabelText(/Manage skills/), { button: 0, ctrlKey: false });
    expect(await screen.findByText(/Could not read the skill catalog/)).toBeInTheDocument();
  });
});

/**
 * QA finding F5, this picker's copy of it — the same defect the Settings list
 * carried, and the same fix: `skills/searchCatalog.ts`, which is the BAAM
 * matcher over this catalog's fields.
 *
 * This list is flat, so the ranking applies to it directly: the rows come out
 * best match first rather than in the catalog's alphabetical order.
 */
describe('BottomMenuSkillSelection search', () => {
  beforeEach(() => {
    mocks.overrides.clear();
    vi.clearAllMocks();
    serve(view());
  });

  const search = (term: string) =>
    fireEvent.change(screen.getByPlaceholderText('Search skills...'), { target: { value: term } });

  it('finds the skills a multi-word phrase names, best match first', async () => {
    serve(view({ skills: [skill('ggplot'), skill('pdf'), skill('r-scripting')] }));
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();

    search('R scripting ggplot visualization');

    await waitFor(() => expect(screen.getAllByRole('menuitemcheckbox')).toHaveLength(2));
    const rows = screen.getAllByRole('menuitemcheckbox');
    // Catalog order is alphabetical — ggplot, pdf, r-scripting — so this is the
    // ranking and not the order the rows arrived in.
    expect(rows[0]).toHaveTextContent('r-scripting');
    expect(rows[1]).toHaveTextContent('ggplot');
  });

  it('holds a one-letter query to whole words', async () => {
    serve(view({ skills: [skill('markdown-render'), skill('r-scripting')] }));
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();

    search('R');

    await waitFor(() => expect(screen.getAllByRole('menuitemcheckbox')).toHaveLength(1));
    expect(screen.getAllByRole('menuitemcheckbox')[0]).toHaveTextContent('r-scripting');
  });

  /**
   * "Enable all" writes every row the filter left on screen, so a filter that
   * returns everything under a one-letter query is a bulk write nobody asked
   * for. The count in the button is the filtered count.
   */
  it('counts only the matched rows in Enable all', async () => {
    serve(
      view({
        skills: [
          skill('markdown-render', { state: { ...skill('x').state, effective: false } }),
          skill('r-scripting', { state: { ...skill('x').state, effective: false } }),
        ],
      })
    );
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();

    search('R');

    expect(await screen.findByRole('button', { name: 'Enable all (1)' })).toBeInTheDocument();
  });
});
