/**
 * Browse skills must agree with what is on disk — for a BUNDLE too.
 *
 * A single skill installs under its SKILL.md frontmatter name
 * (`plan::single_plan`), so `scientific-research` on disk matched the registry
 * id `scientific-research` and rendered INSTALLED. A bundle archive declares no
 * name, so the importer fell back to the staged archive's stem — and the
 * renderer staged every download as `<12 hex nonce>-<asset>.zip`. The
 * marketplace's `single-cell` therefore landed in
 * `~/.config/biorouter/skills/d92c1c985d54-single-cell/`, matched no registry
 * id, kept its checkbox, and could be installed again on every visit.
 *
 * ⚠ **Rendered through BOTH real components, not through a set handed to the
 * modal.** The defect lives in the wiring between them: `SkillsView` builds the
 * installed-name set from the catalog and `BrowseSkillsModal` tests registry ids
 * against it. A test that constructed the set itself would assert the fix
 * against its own copy of the rule and pass whatever the view does — so nothing
 * here is mocked between the catalog the daemon serves and the badge, and the
 * only mock in that path is `loadRegistry`, which is a network fetch.
 */
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SkillsView from './SkillsView';
import type { CatalogBundle, CatalogSkill, CatalogView } from '../../api';

const mocks = vi.hoisted(() => ({
  skillCatalogHandler: vi.fn(),
  refreshSkillCatalog: vi.fn(),
  setSessionSkills: vi.fn(),
  removeSkillPackage: vi.fn(),
  loadRegistry: vi.fn(),
  overrides: new Map<string, boolean>(),
}));

vi.mock('../../api', () => ({
  skillCatalogHandler: (...a: unknown[]) => mocks.skillCatalogHandler(...a),
  refreshSkillCatalog: (...a: unknown[]) => mocks.refreshSkillCatalog(...a),
  setSessionSkills: (...a: unknown[]) => mocks.setSessionSkills(...a),
  removeSkillPackage: (...a: unknown[]) => mocks.removeSkillPackage(...a),
}));

vi.mock('../../store/skillOverrides', () => ({
  loadSkillOverrides: vi.fn(async () => true),
  saveSkillOverrides: vi.fn(async () => undefined),
  setSkillOverride: (name: string, enabled: boolean) => mocks.overrides.set(name, enabled),
  isSkillEnabled: (name: string) => mocks.overrides.get(name) ?? true,
  getSkillOverrides: () => mocks.overrides,
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

// The one fetch on the path. Everything between the catalog and the badge —
// SkillsView's installed-name set and BrowseSkillsModal's `isInstalled` — is
// the real code.
vi.mock('../baam/registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../baam/registry')>()),
  loadRegistry: mocks.loadRegistry,
}));

vi.mock('../Layout/MainPanelLayout', () => ({
  MainPanelLayout: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('../Layout/ReadableContent', () => ({
  ReadableContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
}));
vi.mock('./AddSkillModal', () => ({ default: () => null }));
vi.mock('./CustomSkillModal', () => ({ default: () => null }));

const state = {
  machineEnabled: true,
  session: 'default' as const,
  sessionViaBundle: false,
  hiddenContext: false,
  effective: true,
};

/** A bundle exactly as the catalog reports one installed from the marketplace. */
function installedBundle(directoryName: string, members: string[]): CatalogBundle {
  return {
    name: directoryName,
    // The importer writes the id into `displayName` too when the archive
    // declares no name, which is what Settings renders.
    displayName: directoryName,
    directory: `/skills/${directoryName}`,
    sourceRoot: '/skills',
    source: { kind: 'biorouter', extension: null, label: 'Biorouter' },
    skills: members,
    package: null,
    builtin: false,
    state,
  };
}

function installedSkill(name: string): CatalogSkill {
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
  };
}

const registryEntry = (id: string, name: string, type: string) => ({
  id,
  name,
  category: 'Biomedical' as const,
  type,
  description: `${name} description`,
  tags: [],
  keywords: [],
  download: `https://example.com/${id}.zip`,
  filename: `${id}.zip`,
});

const SINGLE_CELL = registryEntry('single-cell', 'Single-cell', '14 skills · auto-applied');
const SCIENTIFIC = registryEntry(
  'scientific-research',
  'Scientific Research',
  'User-invocable · /scientific-research'
);

function serve(view: Partial<CatalogView>) {
  const full: CatalogView = { generation: 1, roots: [], skills: [], bundles: [], ...view };
  mocks.skillCatalogHandler.mockResolvedValue({ data: full });
  mocks.refreshSkillCatalog.mockResolvedValue({ data: full });
}

/** The row the modal renders for one registry entry. */
function row(name: string) {
  const label = screen
    .getAllByText(name)
    .map((node) => node.closest('label'))
    .find((node): node is HTMLLabelElement => node !== null);
  if (!label) throw new Error(`no Browse skills row for ${name}`);
  return label;
}

async function openBrowse() {
  render(<SkillsView />);
  await screen.findByText('Biorouter Skills (2)');
  await userEvent.click(screen.getByRole('button', { name: 'Browse skills' }));
  await screen.findByText('Browse skills', { selector: 'h2, [role=heading]' });
  await screen.findByText(SINGLE_CELL.description);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.overrides.clear();
  // @ts-expect-error test shim
  window.electron = { openDirectoryInExplorer: vi.fn(), readFile: vi.fn() };
  mocks.loadRegistry.mockResolvedValue({
    registry: { version: 1, skills: [SINGLE_CELL, SCIENTIFIC] },
    live: true,
    fetchedAt: '2026-09-13T00:00:00Z',
  });
});

describe('Browse skills and the installed catalog agree about a bundle', () => {
  /**
   * The blocker, stated as the user meets it: the bundle IS installed — it is
   * the `Biorouter Skills (2)` row one click away — and the modal offered it
   * again because the directory it landed in was `d92c1c985d54-single-cell`.
   */
  it('marks a bundle installed under a staging-nonce directory as installed', async () => {
    serve({
      skills: [installedSkill('scientific-research')],
      bundles: [
        installedBundle('d92c1c985d54-single-cell', ['single-cell-clustering', 'single-cell-io']),
      ],
    });

    await openBrowse();

    const bundleRow = row('Single-cell');
    expect(within(bundleRow).getByText('Installed')).toBeInTheDocument();
    expect(bundleRow.querySelector('input[type=checkbox]')).toBeDisabled();
  });

  /**
   * The half that always worked, kept beside it: a single skill installs under
   * its frontmatter name and has to keep reading installed. Without this, a
   * "fix" that marked every registry row installed would pass the case above.
   */
  it('still marks a single skill installed under its plain id', async () => {
    serve({
      skills: [installedSkill('scientific-research')],
      bundles: [
        installedBundle('d92c1c985d54-single-cell', ['single-cell-clustering', 'single-cell-io']),
      ],
    });

    await openBrowse();

    const singleRow = row('Scientific Research');
    expect(within(singleRow).getByText('Installed')).toBeInTheDocument();
    expect(singleRow.querySelector('input[type=checkbox]')).toBeDisabled();
  });

  /**
   * The false positive the narrow rule exists to refuse. Widen
   * `withoutStagingNonce` to "strip whatever precedes the first dash" — the
   * obvious cheap version — and an unrelated package called
   * `chip-single-cell` starts reporting the marketplace's `single-cell` as
   * installed, and the user can no longer install it at all. Twelve hex digits
   * exactly is what separates the nonce this app wrote from a package name that
   * happens to contain a dash.
   */
  it('does not read an unrelated package name as a de-nonced registry id', async () => {
    serve({
      skills: [],
      bundles: [installedBundle('chip-single-cell', ['chip-member'])],
    });

    render(<SkillsView />);
    await screen.findByText('Biorouter Skills (1)');
    await userEvent.click(screen.getByRole('button', { name: 'Browse skills' }));
    await screen.findByText(SINGLE_CELL.description);

    const entry = row('Single-cell');
    expect(within(entry).queryByText('Installed')).not.toBeInTheDocument();
    expect(entry.querySelector('input[type=checkbox]')).not.toBeDisabled();
  });

  /**
   * And the negative: nothing on disk resembling a registry id leaves both rows
   * selectable. This is the assertion that would have to fail for the two above
   * to be measuring the badge rather than a component that renders "Installed"
   * unconditionally.
   */
  it('leaves a registry entry selectable when nothing on disk matches it', async () => {
    serve({
      skills: [installedSkill('some-other-skill')],
      bundles: [installedBundle('a1b2c3d4e5f6-unrelated-pack', ['member-one'])],
    });

    render(<SkillsView />);
    await screen.findByText('Biorouter Skills (2)');
    await userEvent.click(screen.getByRole('button', { name: 'Browse skills' }));
    await screen.findByText(SINGLE_CELL.description);

    for (const name of ['Single-cell', 'Scientific Research']) {
      const entry = row(name);
      expect(within(entry).queryByText('Installed')).not.toBeInTheDocument();
      expect(entry.querySelector('input[type=checkbox]')).not.toBeDisabled();
    }
  });
});
