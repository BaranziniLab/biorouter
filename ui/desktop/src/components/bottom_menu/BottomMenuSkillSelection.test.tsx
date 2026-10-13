import * as React from 'react';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SkillsPanel, useSkillPicker } from './BottomMenuSkillSelection';
import { SKILLS_COPY, TOOLS_COPY } from './copy';
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

/**
 * The skills list as the Tools chip holds it (spec 3.7). The harness's button
 * stands in for the chip and states the hook's count, so the count assertions
 * read one label.
 */
function BottomMenuSkillSelection({ sessionId }: { sessionId: string | null }) {
  const [open, setOpen] = React.useState(false);
  const picker = useSkillPicker(sessionId, { open });
  return (
    <>
      <button
        type="button"
        aria-label={`Manage skills (${picker.activeCount} enabled)`}
        data-answered={String(picker.answered)}
        onClick={() => setOpen((current) => !current)}
      />
      {open && <SkillsPanel picker={picker} />}
    </>
  );
}

async function openMenu() {
  fireEvent.click(screen.getByLabelText(/Manage skills/));
  return screen.findAllByRole('option');
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
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'false'));
  });

  it('raises no success toast, and rolls back with an error when the backend refuses', async () => {
    mocks.setSessionSkills.mockRejectedValue(new Error('session not found'));

    render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    fireEvent.click(toggle);

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    expect(mocks.toastError.mock.calls[0][0].msg).toContain('session not found');
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));
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
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));
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
    await waitFor(() => expect(beta).toHaveAttribute('aria-selected', 'true'));
    expect(alpha).toHaveAttribute('aria-selected', 'false');
  });

  /**
   * Decision 6.9: the switch is the confirmation, so a confirmed write raises
   * no toast. Who the change reaches is said by the scope line instead, before
   * the click rather than after it.
   */
  it('says "This chat" only when there is a chat, and toasts nothing on success', async () => {
    mocks.setSessionSkills.mockResolvedValue({
      data: { catalog: view(), sessionAdd: [], sessionRemove: ['example-skill'] },
    });
    const { unmount } = render(<BottomMenuSkillSelection sessionId="20260824_1" />);
    const [toggle] = await openMenu();
    expect(screen.getByText(TOOLS_COPY.scope.thisChat)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: `About ${TOOLS_COPY.scope.about}` })
    ).toHaveAccessibleDescription(SKILLS_COPY.helpChat);
    fireEvent.click(toggle);
    await waitFor(() => expect(mocks.setSessionSkills).toHaveBeenCalledTimes(1));
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    unmount();

    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();
    expect(screen.getByText(TOOLS_COPY.scope.newChats)).toBeInTheDocument();
    expect(screen.queryByText(TOOLS_COPY.scope.thisChat)).not.toBeInTheDocument();
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

  /**
   * UI defect 3 (final check, 2026-10-12): each row's switch is named by the
   * row's name alone (not its badge, its source or a bundle's count), as every
   * switch on the Extensions page is, and it stays out of the accessibility tree
   * so a screen reader hears each row once (the option).
   */
  it('names each row’s switch by the row’s name, and keeps the row the one control', async () => {
    const catalog = (exampleOn: boolean) =>
      view({
        skills: [
          skill('example-skill', {
            state: {
              machineEnabled: exampleOn,
              session: 'default',
              sessionViaBundle: false,
              hiddenContext: false,
              effective: exampleOn,
            },
          }),
          skill('r-scripting', { builtin: true }),
          skill('word', {
            sourceRoot: '/extensions/BiorOffice/skills',
            source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
          }),
        ],
        bundles: [
          bundle('hyperframes', ['hyperframes', 'media-use'], { displayName: 'HyperFrames' }),
        ],
      });
    serve(catalog(true));
    render(<BottomMenuSkillSelection sessionId={null} />);
    const rows = await openMenu();
    expect(rows).toHaveLength(4);
    expect(screen.queryAllByRole('switch')).toHaveLength(0);
    expect(screen.getAllByRole('switch', { hidden: true })).toHaveLength(4);
    for (const name of ['example-skill', 'r-scripting', 'word', 'HyperFrames']) {
      const picture = screen.getByRole('switch', { name, hidden: true });
      expect(picture.closest('[role="option"]')).toHaveAttribute('aria-selected', 'true');
      expect(picture).toHaveAttribute('aria-checked', 'true');
    }

    // The row is still what a click toggles, and the named switch follows it (the
    // machine-wide write reads the catalog back, so the next read says off).
    serve(catalog(false));
    fireEvent.click(
      screen
        .getByRole('switch', { name: 'example-skill', hidden: true })
        .closest('[role="option"]')!
    );
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: 'example-skill', hidden: true })).toHaveAttribute(
        'aria-checked',
        'false'
      )
    );
    expect(mocks.overrides.get('example-skill')).toBe(false);
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

  it('shows a package bundle as one row with its member count, members in its tooltip', async () => {
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
    expect(screen.getByText(SKILLS_COPY.bundleCount(2))).toBeInTheDocument();
    // The members are one hover away; no entry-point subline (spec 3.7).
    expect(screen.queryByText(/entry point/)).not.toBeInTheDocument();
    expect(SKILLS_COPY.bundleMembers(['hyperframes', 'media-use'])).toBe(
      'Includes hyperframes, media-use'
    );
  });

  // -------------------------------------------------------------- hub (machine)

  it('applies rapid hub choices in order and leaves the final choice enabled', async () => {
    render(<BottomMenuSkillSelection sessionId={null} />);
    const [toggle] = await openMenu();
    expect(toggle).toHaveAttribute('aria-selected', 'true');

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
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'false'));

    serve(view());
    fireEvent.click(toggle);
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));
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
    await waitFor(() => expect(toggle).toHaveAttribute('aria-selected', 'true'));
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
    fireEvent.click(screen.getByLabelText(/Manage skills/));
    expect(await screen.findByText(/Could not read the skill catalog/)).toBeInTheDocument();
  });

  /**
   * V2-R7-3: the Tools chip shows its number only once every list has answered
   * for THIS chat. The catalog hook keeps its previous view until the next
   * chat's read commits, so `answered` must not read that view as this chat's.
   */
  describe('answered for this chat (V2-R7-3)', () => {
    const twoSkills = view({ skills: [skill('alpha'), skill('beta')] });

    it('has not answered until the chat’s catalog lands', async () => {
      let resolveRead: ((value: unknown) => void) | undefined;
      mocks.skillCatalogHandler.mockReturnValue(
        new Promise((resolve) => {
          resolveRead = resolve;
        })
      );
      render(<BottomMenuSkillSelection sessionId="chat-1" />);
      const chip = screen.getByLabelText(/Manage skills/);
      expect(chip).toHaveAttribute('data-answered', 'false');

      await act(async () => {
        resolveRead?.({ data: twoSkills });
        await Promise.resolve();
      });
      expect(chip).toHaveAttribute('data-answered', 'true');
      expect(chip).toHaveAccessibleName('Manage skills (2 enabled)');
    });

    it('has answered for a catalog that resolves before React renders the read', async () => {
      // The read's `loading: true` and its answer land in one batch here, so a
      // check that waited to see `loading` go true would never answer.
      render(<BottomMenuSkillSelection sessionId="chat-1" />);
      await waitFor(() =>
        expect(screen.getByLabelText(/Manage skills/)).toHaveAttribute('data-answered', 'true')
      );
    });

    it('has not answered for the next chat until its own catalog lands', async () => {
      serve(twoSkills);
      const { rerender } = render(<BottomMenuSkillSelection sessionId="chat-a" />);
      const chip = screen.getByLabelText(/Manage skills/);
      await waitFor(() => expect(chip).toHaveAttribute('data-answered', 'true'));
      expect(chip).toHaveAccessibleName('Manage skills (2 enabled)');

      let resolveNext: ((value: unknown) => void) | undefined;
      mocks.skillCatalogHandler.mockReturnValue(
        new Promise((resolve) => {
          resolveNext = resolve;
        })
      );
      rerender(<BottomMenuSkillSelection sessionId="chat-b" />);
      // In the very commit: chat A's catalog is still the hook's view.
      expect(chip).toHaveAttribute('data-answered', 'false');
      await act(async () => {});
      expect(chip).toHaveAttribute('data-answered', 'false');

      await act(async () => {
        resolveNext?.({ data: view({ skills: [skill('gamma')] }) });
        await Promise.resolve();
      });
      expect(chip).toHaveAttribute('data-answered', 'true');
      expect(chip).toHaveAccessibleName('Manage skills (1 enabled)');
    });

    it('has answered when the catalog cannot be read', async () => {
      mocks.skillCatalogHandler.mockRejectedValue(new Error('daemon is down'));
      render(<BottomMenuSkillSelection sessionId="chat-1" />);
      await waitFor(() =>
        expect(screen.getByLabelText(/Manage skills/)).toHaveAttribute('data-answered', 'true')
      );
    });
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
    fireEvent.change(screen.getByPlaceholderText(SKILLS_COPY.search), { target: { value: term } });

  it('finds the skills a multi-word phrase names, best match first', async () => {
    serve(view({ skills: [skill('ggplot'), skill('pdf'), skill('r-scripting')] }));
    render(<BottomMenuSkillSelection sessionId={null} />);
    await openMenu();

    search('R scripting ggplot visualization');

    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2));
    const rows = screen.getAllByRole('option');
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

    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(1));
    expect(screen.getAllByRole('option')[0]).toHaveTextContent('r-scripting');
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
