import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SkillsView from './SkillsView';
import type { CatalogBundle, CatalogSkill, CatalogView } from '../../api';
import { DEFAULT_MIN_SEARCH_LENGTH } from '../conversation/SearchBar';

const mocks = vi.hoisted(() => ({
  skillCatalogHandler: vi.fn(),
  refreshSkillCatalog: vi.fn(),
  setSessionSkills: vi.fn(),
  removeSkillPackage: vi.fn(),
  saveSkillOverrides: vi.fn(async () => undefined),
  loadSkillOverrides: vi.fn(async () => true),
  overrides: new Map<string, boolean>(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  skillCatalogHandler: (...a: unknown[]) => mocks.skillCatalogHandler(...a),
  refreshSkillCatalog: (...a: unknown[]) => mocks.refreshSkillCatalog(...a),
  setSessionSkills: (...a: unknown[]) => mocks.setSessionSkills(...a),
  removeSkillPackage: (...a: unknown[]) => mocks.removeSkillPackage(...a),
}));

vi.mock('../../store/skillOverrides', () => ({
  loadSkillOverrides: mocks.loadSkillOverrides,
  saveSkillOverrides: mocks.saveSkillOverrides,
  setSkillOverride: (name: string, enabled: boolean) => mocks.overrides.set(name, enabled),
  isSkillEnabled: (name: string) => mocks.overrides.get(name) ?? true,
  getSkillOverrides: () => mocks.overrides,
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

vi.mock('../Layout/MainPanelLayout', () => ({
  MainPanelLayout: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('../Layout/ReadableContent', () => ({
  ReadableContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
// The real SearchView owns a cmd-F overlay and a scroll-area contract; all the
// view reads from it is the term it reports, so the mock is that one wire —
// without it the search-filtered branches below are unreachable from a test.
//
// ⚠ **The mock carries the MINIMUM-LENGTH floor, because the real bar does.**
// It used to hand every keystroke straight through, so the `R` tests below were
// green while measuring nothing: in the app a one-character query never reached
// this view at all — `SearchBar` reported an empty term and the whole catalog
// rendered under its provenance headings. The floor is imported rather than
// retyped so the mock cannot drift from the component it stands in for, and the
// bar's own half of the contract is asserted directly in `SearchBar.test.tsx`.
const searchMocks = vi.hoisted(() => ({ defaultFloor: 2 }));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({
    children,
    onSearch,
    minSearchLength = searchMocks.defaultFloor,
  }: {
    children: React.ReactNode;
    onSearch: (term: string, caseSensitive: boolean) => void;
    minSearchLength?: number;
  }) => (
    <div>
      <input
        aria-label="Search skills"
        onChange={(event) =>
          onSearch(event.target.value.length >= minSearchLength ? event.target.value : '', false)
        }
      />
      {children}
    </div>
  ),
}));
vi.mock('../baam/BrowseSkillsModal', () => ({ default: () => null }));
vi.mock('./AddSkillModal', () => ({ default: () => null }));
vi.mock('./CustomSkillModal', () => ({ default: () => null }));

const state = {
  machineEnabled: true,
  session: 'default' as const,
  sessionViaBundle: false,
  hiddenContext: false,
  effective: true,
};

function skill(name: string, overrides: Partial<CatalogSkill> = {}): CatalogSkill {
  return {
    name,
    description: `${name} does things`,
    slug: name,
    directory: `/skills/${name}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    bundle: null,
    builtin: false,
    state,
    ...overrides,
  };
}

function bundle(
  name: string,
  members: string[],
  overrides: Partial<CatalogBundle> = {}
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
    state,
    ...overrides,
  };
}

function serve(view: Partial<CatalogView>) {
  const full: CatalogView = { generation: 1, roots: [], skills: [], bundles: [], ...view };
  mocks.skillCatalogHandler.mockResolvedValue({ data: full });
  mocks.refreshSkillCatalog.mockResolvedValue({ data: full });
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.overrides.clear();
  // @ts-expect-error test shim
  window.electron = { openDirectoryInExplorer: vi.fn(), readFile: vi.fn() };
  serve({ skills: [skill('my-skill')] });
});

describe('SkillsView', () => {
  /**
   * The visible half of #113 root cause 2: BiorOffice's bundled skills were
   * loaded by the model and had no row here, because this view scanned three
   * roots against the backend's seven.
   */
  it('lists extension-bundled skills in their own group', async () => {
    serve({
      skills: [
        skill('my-skill'),
        skill('word', {
          sourceRoot: '/extensions/BiorOffice/skills',
          source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
        }),
      ],
    });
    render(<SkillsView />);
    expect(await screen.findByText('From BiorOffice (1)')).toBeInTheDocument();
    expect(screen.getByText('Biorouter Skills (1)')).toBeInTheDocument();
  });

  /**
   * A skill an extension supplies is not the user's to delete — the extension
   * would put it back. Same lesson as the built-in badge, second case.
   */
  it('offers no Delete for a skill an installed extension supplies', async () => {
    serve({
      skills: [
        skill('word', {
          sourceRoot: '/extensions/BiorOffice/skills',
          source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
        }),
      ],
    });
    render(<SkillsView />);
    await screen.findByText('word');
    expect(screen.queryByLabelText('Delete word')).not.toBeInTheDocument();
  });

  it('shows a package as one expandable row, and opens to its components', async () => {
    serve({
      skills: [
        skill('hyperframes', { bundle: 'hyperframes', slug: 'hyperframes/hyperframes' }),
        skill('media-use', { bundle: 'hyperframes', slug: 'hyperframes/media-use' }),
      ],
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
            groups: { core: ['hyperframes'], 'on-demand': ['media-use'] },
          },
        }),
      ],
    });
    render(<SkillsView />);

    expect(await screen.findByText('HyperFrames')).toBeInTheDocument();
    expect(screen.getByText('Biorouter Skills (1)')).toBeInTheDocument();
    expect(screen.getByText('entry point: hyperframes')).toBeInTheDocument();

    // Collapsed, the row summarises; expanded, it details.
    fireEvent.click(screen.getByLabelText('Expand HyperFrames'));
    const list = await screen.findByRole('list');
    const items = within(list).getAllByRole('listitem');
    expect(items).toHaveLength(2);
    expect(items[1]).toHaveTextContent('media-use');
    expect(items[1]).toHaveTextContent('[on-demand]');
    expect(items[0]).toHaveTextContent('→');
  });

  /// Skill names are PROSE and must be set in the body font.
  ///
  /// The collapsed member list was `font-mono` while "entry point: …" three
  /// lines above it — printing one of those very same names — was body. One
  /// string ("hyperframes"), two typefaces, in one card, both on screen at
  /// once. Expanding the row then rendered the same names in the body font a
  /// third way, so the face flipped on expand too.
  ///
  /// jsdom never runs Tailwind, so a computed-style assertion would pass
  /// whatever the class says. This asserts the CLASS, and walks the ancestors
  /// because `font-mono` on a parent is inherited — which is how this would
  /// regress without the element itself being touched.
  it('sets collapsed package member names in the body font, not monospace', async () => {
    serve({
      skills: [
        skill('hyperframes', { bundle: 'hyperframes', slug: 'hyperframes/hyperframes' }),
        skill('media-use', { bundle: 'hyperframes', slug: 'hyperframes/media-use' }),
      ],
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
            groups: { core: ['hyperframes'], 'on-demand': ['media-use'] },
          },
        }),
      ],
    });
    render(<SkillsView />);

    const members = await screen.findByText('hyperframes · media-use');
    // The same name, in the same card, is already body font here.
    expect(screen.getByText('entry point: hyperframes').className).not.toMatch(/font-mono/);

    expect(members.className).not.toMatch(/font-mono/);
    for (let node = members.parentElement; node; node = node.parentElement) {
      expect(node.className ?? '').not.toMatch(/font-mono/);
      if (node.tagName === 'BODY') break;
    }
  });

  it('keeps same-named package members scoped to their physical root', async () => {
    const projectRoot = '/project/.biorouter/skills';
    serve({
      skills: [
        skill('alpha', { bundle: 'pack', slug: 'pack/alpha' }),
        skill('beta', {
          bundle: 'pack',
          slug: 'pack/beta',
          directory: `${projectRoot}/pack/beta`,
          sourceRoot: projectRoot,
          source: { kind: 'project', extension: null, label: 'Project' },
        }),
      ],
      bundles: [
        bundle('pack', ['alpha'], { displayName: 'Installed Pack' }),
        bundle('pack', ['beta'], {
          displayName: 'Project Pack',
          directory: `${projectRoot}/pack`,
          sourceRoot: projectRoot,
          source: { kind: 'project', extension: null, label: 'Project' },
        }),
      ],
    });
    render(<SkillsView />);

    fireEvent.click(await screen.findByLabelText('Expand Installed Pack'));
    const installed = screen.getByText('Installed Pack').closest('.biorouter-list-row')!;
    expect(within(installed as HTMLElement).getByRole('list')).toHaveTextContent('alpha');
    expect(within(installed as HTMLElement).getByRole('list')).not.toHaveTextContent('beta');

    fireEvent.click(screen.getByLabelText('Expand Project Pack'));
    const project = screen.getByText('Project Pack').closest('.biorouter-list-row')!;
    expect(within(project as HTMLElement).getByRole('list')).toHaveTextContent('beta');
    expect(within(project as HTMLElement).getByRole('list')).not.toHaveTextContent('alpha');
  });

  it('removes a package through the importer rather than deleting a directory', async () => {
    serve({
      skills: [skill('alpha', { bundle: 'pack', slug: 'pack/alpha' })],
      bundles: [bundle('pack', ['alpha'])],
    });
    mocks.removeSkillPackage.mockResolvedValue({ data: { id: 'pack' } });
    render(<SkillsView />);

    fireEvent.click(await screen.findByLabelText('Delete skill package pack'));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete package' }));

    await waitFor(() => expect(mocks.removeSkillPackage).toHaveBeenCalledTimes(1));
    expect(mocks.removeSkillPackage.mock.calls[0][0].body).toEqual({
      id: 'pack',
      sourceRoot: '/skills',
    });
  });

  /**
   * A skill's directory name and its declared name are allowed to differ, and
   * the frontmatter is what wins for identity — so removal must use the
   * INSTALLED directory or it would miss the folder entirely.
   */
  it('removes a single skill by its installed directory, not its declared name', async () => {
    serve({
      skills: [skill('gwas-pipeline', { slug: 'run-gwas', directory: '/skills/run-gwas' })],
    });
    mocks.removeSkillPackage.mockResolvedValue({ data: { id: 'run-gwas' } });
    render(<SkillsView />);

    fireEvent.click(await screen.findByLabelText('Delete gwas-pipeline'));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(mocks.removeSkillPackage).toHaveBeenCalledTimes(1));
    expect(mocks.removeSkillPackage.mock.calls[0][0].body.id).toBe('run-gwas');
  });

  it('reports a catalog it could not read instead of claiming there are no skills', async () => {
    mocks.skillCatalogHandler.mockRejectedValue(new Error('daemon is down'));
    mocks.refreshSkillCatalog.mockRejectedValue(new Error('daemon is down'));
    render(<SkillsView />);
    expect(await screen.findByText(/Could not read the skill catalog/)).toBeInTheDocument();
    // The failure is the only thing said. An empty state beside it would be the
    // "there are no skills" claim this test is named for, in a second voice.
    expect(screen.queryByRole('region', { name: 'No skills yet' })).not.toBeInTheDocument();
  });

  /** The page title is the shared header's `<h1>`, not a heading of this view's own. */
  it('titles the page once, at level 1', async () => {
    render(<SkillsView />);
    await screen.findByText('my-skill');
    expect(screen.getByRole('heading', { level: 1, name: 'Skills' })).toBeInTheDocument();
  });
});

/**
 * The empty, loading and search-empty branches, which were one `<p>` holding
 * four different sentences.
 *
 * ⚠ The skeletons are keyed on an EMPTY list, not on `loading` alone, and the
 * last test here is what pins that. `reload` raises `loading` after every
 * install, delete and `catalog:changed` rescan as well as on first load — so a
 * branch that asked only whether a load was in flight would replace the whole
 * list with placeholders every time a package was removed. That is the defect
 * `SchedulesView` shipped and had to be rescued from; it is cheaper to assert it
 * here than to rediscover it.
 */
describe('SkillsView empty and loading states', () => {
  const skeletons = () => document.querySelectorAll('[data-slot="skeleton"]');

  it('offers a way out of an empty catalog instead of a bare line of prose', async () => {
    serve({});
    render(<SkillsView />);

    const empty = await screen.findByRole('region', { name: 'No skills yet' });
    expect(within(empty).getByRole('button', { name: 'Add skill' })).toBeInTheDocument();
  });

  it('says a search matched nothing without claiming the catalog is empty', async () => {
    serve({ skills: [skill('alpha')] });
    render(<SkillsView />);
    await screen.findByText('alpha');

    fireEvent.change(screen.getByLabelText('Search skills'), { target: { value: 'zzz' } });

    expect(await screen.findByRole('region', { name: 'No matching skills' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'No skills yet' })).not.toBeInTheDocument();
  });

  it('shows rows in the shape of rows while the catalog first loads', async () => {
    mocks.skillCatalogHandler.mockReturnValue(new Promise(() => {}));
    render(<SkillsView />);

    await waitFor(() => expect(skeletons().length).toBeGreaterThan(0));
    expect(screen.queryByRole('region', { name: 'No skills yet' })).not.toBeInTheDocument();
  });

  it('keeps the rows on screen while a rescan is in flight', async () => {
    serve({ skills: [skill('alpha')] });
    mocks.removeSkillPackage.mockResolvedValue({ data: { id: 'alpha' } });
    // A rescan that never settles, which is what a delete leaves in flight.
    mocks.refreshSkillCatalog.mockReturnValue(new Promise(() => {}));
    render(<SkillsView />);

    fireEvent.click(await screen.findByLabelText('Delete alpha'));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));

    await waitFor(() => expect(mocks.removeSkillPackage).toHaveBeenCalledTimes(1));
    expect(screen.getByText('alpha')).toBeInTheDocument();
    expect(skeletons()).toHaveLength(0);
  });
});

/**
 * ⚠ **A seeded BUNDLE must offer no Delete, and its own field is the only
 * thing that can say so.** `SkillItem` gates its Trash on `CatalogSkill.builtin`
 * — the daemon's answer, not a list in the renderer — but a bundle row is a
 * different control over a different directory, and it had no such gate. So a
 * shipped bundle rendered a working Trash: `removeSkillPackage` succeeded, the
 * toast confirmed it, and the next startup rewrote the folder. That is exactly
 * regression 1 of #77, one level up from where it was fixed.
 *
 * `CatalogBundle.builtin` is `is_shipped_entry_name` on the Rust side, so this
 * cannot drift from the seeder.
 */
describe('SkillsView built-in bundles', () => {
  it('offers no Delete on a bundle the daemon says it shipped', async () => {
    serve({
      skills: [skill('member', { bundle: 'shipped-bundle', builtin: true })],
      bundles: [bundle('shipped-bundle', ['member'], { builtin: true })],
    });
    render(<SkillsView />);

    const row = (await screen.findByText('shipped-bundle')).closest('.biorouter-list-row')!;
    expect(
      within(row as HTMLElement).queryByLabelText(/Delete skill package/)
    ).not.toBeInTheDocument();
    expect(within(row as HTMLElement).getByText('Built-in')).toBeInTheDocument();
  });

  /**
   * The control is present for an installed package, so the assertion above is
   * about built-in-ness and not about bundle rows in general.
   */
  it('still offers Delete on an installed package', async () => {
    serve({
      skills: [skill('media-use', { bundle: 'hyperframes' })],
      bundles: [bundle('hyperframes', ['media-use'])],
    });
    render(<SkillsView />);

    const row = (await screen.findByText('hyperframes')).closest('.biorouter-list-row')!;
    expect(within(row as HTMLElement).getByLabelText(/Delete skill package/)).toBeInTheDocument();
  });
});

/**
 * QA finding F5, this view's copy of it.
 *
 * The filter asked whether the WHOLE lowercased query occurred inside one
 * field, so a phrase naming two installed skills matched neither of them, and
 * a one-letter query matched every skill whose prose contained that letter.
 * Both are measured below on the rows the daemon serves; the matcher they now
 * go through is `searchCatalog.ts`, which is `baam/search.ts` with this
 * catalog's fields.
 */
describe('SkillsView search', () => {
  const search = (term: string) =>
    fireEvent.change(screen.getByLabelText('Search skills'), { target: { value: term } });

  it("stands in for the bar with the bar's own default floor", () => {
    // The mock cannot import the constant — its factory is hoisted above the
    // imports — so the two are pinned here instead. Without this the default
    // could move and the one-letter tests below would go green again while
    // measuring a floor the app does not have.
    expect(DEFAULT_MIN_SEARCH_LENGTH).toBe(searchMocks.defaultFloor);
  });

  it('finds the skills a multi-word phrase names, best match first', async () => {
    serve({ skills: [skill('ggplot'), skill('pdf'), skill('r-scripting')] });
    render(<SkillsView />);
    await screen.findByText('ggplot');

    search('R scripting ggplot visualization');

    // Both are named by the query; before this change the whole phrase was
    // looked for as a substring and neither row survived.
    expect(await screen.findByText('r-scripting')).toBeInTheDocument();
    expect(screen.getByText('ggplot')).toBeInTheDocument();
    expect(screen.queryByText('pdf')).not.toBeInTheDocument();

    // One ranked list under a query, not the provenance groups: `r-scripting`
    // matches two of the query's terms and `ggplot` one, and a heading would
    // have ordered them alphabetically instead.
    const matches = screen.getByRole('heading', { level: 2, name: /Matches \(2\)/ }).parentElement!;
    const text = matches.textContent ?? '';
    expect(text.indexOf('r-scripting')).toBeLessThan(text.indexOf('ggplot'));
    expect(screen.queryByText(/Biorouter Skills/)).not.toBeInTheDocument();
  });

  it('holds a one-letter query to whole words', async () => {
    serve({ skills: [skill('markdown-render'), skill('r-scripting')] });
    render(<SkillsView />);
    await screen.findByText('r-scripting');

    search('R');

    expect(
      await screen.findByRole('heading', { level: 2, name: /Matches \(1\)/ })
    ).toBeInTheDocument();
    expect(screen.getByText('r-scripting')).toBeInTheDocument();
    // `markdown-render` holds the letter twice and means nothing by it.
    expect(screen.queryByText('markdown-render')).not.toBeInTheDocument();
  });

  /**
   * The defect this view had until the floor became per-surface: `SearchBar`
   * refused anything under two characters and reported an EMPTY term, which
   * this view reads as "browsing". Measured in the running app before the fix,
   * on a two-skill catalog: `z` showed both rows under `FROM THIS PROJECT (2)`
   * while `zz` correctly showed "No matching skills" — a control that looks
   * like it filtered and did not.
   */
  it('answers a one-letter query that matches nothing, instead of showing everything', async () => {
    serve({ skills: [skill('markdown-render'), skill('r-scripting')] });
    render(<SkillsView />);
    await screen.findByText('r-scripting');

    search('z');

    expect(await screen.findByText('No matching skills')).toBeInTheDocument();
    expect(screen.queryByText('r-scripting')).not.toBeInTheDocument();
    expect(screen.queryByText(/Biorouter Skills/)).not.toBeInTheDocument();
  });

  it('keeps the provenance groups when nothing is typed', async () => {
    serve({
      skills: [
        skill('my-skill'),
        skill('word', {
          sourceRoot: '/extensions/BiorOffice/skills',
          source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
        }),
      ],
    });
    render(<SkillsView />);

    expect(await screen.findByText('Biorouter Skills (1)')).toBeInTheDocument();
    expect(screen.getByText('From BiorOffice (1)')).toBeInTheDocument();
    expect(screen.queryByText(/Matches \(/)).not.toBeInTheDocument();
  });

  /**
   * The Delete a row offers follows the ROW's own source, not the heading it
   * happens to sit under — which is the thing one flat ranked list could
   * quietly lose, since `fromExtension` used to be a property of the group.
   */
  it('still offers no Delete for an extension-supplied skill inside the matches list', async () => {
    serve({
      skills: [
        skill('r-scripting'),
        skill('r-plotting', {
          sourceRoot: '/extensions/BiorOffice/skills',
          source: { kind: 'extension', extension: 'BiorOffice', label: 'BiorOffice' },
        }),
      ],
    });
    render(<SkillsView />);
    await screen.findByText('r-scripting');

    search('R');

    expect(await screen.findByText('r-plotting')).toBeInTheDocument();
    expect(screen.getByLabelText('Delete r-scripting')).toBeInTheDocument();
    expect(screen.queryByLabelText('Delete r-plotting')).not.toBeInTheDocument();
  });
});
