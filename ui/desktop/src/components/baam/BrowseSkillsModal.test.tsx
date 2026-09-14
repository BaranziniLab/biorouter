import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { MARKETPLACE_SKILLS } from './marketplace.fixture';

const mocks = vi.hoisted(() => {
  // Error toasts, as react-toastify tracks them: an id is active from the
  // moment it is raised until something dismisses it.
  const activeToasts = new Set<string>();
  return {
    loadRegistry: vi.fn(),
    activeToasts,
    toastError: vi.fn(({ title, msg }: { title: string; msg: string }) => {
      const id = `error:${title}:${msg}`;
      activeToasts.add(id);
      return id;
    }),
    dismiss: vi.fn((id: string) => activeToasts.delete(id)),
  };
});

vi.mock('./registry', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./registry')>()),
  loadRegistry: mocks.loadRegistry,
}));
vi.mock('./installSkill', () => ({ installRegistrySkill: vi.fn() }));
vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: mocks.toastError }));
vi.mock('react-toastify', () => ({
  toast: {
    isActive: (id: string) => mocks.activeToasts.has(id),
    dismiss: mocks.dismiss,
  },
}));

import BrowseSkillsModal from './BrowseSkillsModal';
import { installButtonLabel } from './installCopy';
import { resetInstallReport } from './installReport';
import { installRegistrySkill, type InstallResult } from './installSkill';
import type { RegistrySkill } from './registry';
import { toastError, toastSuccess } from '../../toasts';

const skill = {
  id: 'scientific-research',
  name: 'scientific-research',
  category: 'Core' as const,
  // A real registry value: prose, not a token. See landing/registry.json.
  type: 'User-invocable · /scientific-research',
  description: 'Research workflows',
  tags: [],
  keywords: [],
  download: 'https://example.com/scientific-research.zip',
  filename: 'scientific-research.zip',
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.activeToasts.clear();
  resetInstallReport();
  mocks.loadRegistry.mockResolvedValue({
    registry: { version: 1, skills: [skill] },
    live: true,
    fetchedAt: '2026-09-02T00:00:00Z',
  });
});

/// `skill.type` is an English phrase, not a machine token. The real registry
/// ships values like "5 skills · auto-applied" and
/// "User-invocable · /scientific-research" — and this span sits inline, on the
/// same row, beside `skill.name` in the body font. Monospace here was the
/// reverse defect: prose set in the code face.
///
/// jsdom never runs Tailwind, so asserting a computed font would pass whatever
/// the class says. This asserts the CLASS, and walks the ancestors because
/// `font-mono` on a parent is inherited.
describe('BrowseSkillsModal — a classification phrase is prose, not a token', () => {
  it('sets the skill type in the body font, not monospace', async () => {
    render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);

    const type = await screen.findByText(skill.type);
    expect(type.className).not.toMatch(/font-mono/);
    for (let node = type.parentElement; node; node = node.parentElement) {
      expect(node.className ?? '').not.toMatch(/font-mono/);
      if (node.tagName === 'BODY') break;
    }
  });
});

/// The button reads "Install  skills" — two spaces — whenever nothing is
/// selected, which is the state the dialog OPENS in. The old template put the
/// count between two literal spaces (`` `Install ${n > 0 ? n : ''} skill…` ``),
/// so the empty substitution collapsed to nothing and left the pair behind.
///
/// Tested on the pure label rather than only through a render: the arithmetic
/// is a function of one number, and a threshold you can exercise only by
/// mounting a component is one nobody re-tests.
describe('BrowseSkillsModal — the install label has one space between words', () => {
  it.each([
    [0, 'Install skills'],
    [1, 'Install 1 skill'],
    [2, 'Install 2 skills'],
    [3, 'Install 3 skills'],
  ])('reads correctly with %i selected', (count, expected) => {
    expect(installButtonLabel(count)).toBe(expected);
  });

  it('never emits a double space at any count', () => {
    for (let n = 0; n <= 12; n += 1) {
      expect(installButtonLabel(n)).not.toMatch(/ {2}/);
      expect(installButtonLabel(n).trim()).toBe(installButtonLabel(n));
    }
  });

  /// The one shape the user actually hits first, asserted through the real
  /// DOM so the helper cannot be correct while the button ignores it.
  it('renders "Install skills" in the freshly-opened dialog', async () => {
    render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);

    const button = await screen.findByRole('button', { name: 'Install skills' });
    expect(button).toBeDisabled();
    // `textContent` rather than the accessible name, which normalises runs of
    // whitespace and would report the bug as fixed while it was still there.
    expect(button.textContent).toBe('Install skills');
  });
});

/** The skill names the list shows, top to bottom: one checkbox per row. */
function shownSkillNames(): string[] {
  return screen
    .queryAllByRole('checkbox')
    .map((box) => box.closest('label')?.querySelector('span')?.textContent ?? '');
}

async function openWithMarketplaceSkills() {
  mocks.loadRegistry.mockResolvedValue({
    registry: { version: 2, source: 'test', extensions: [], skills: MARKETPLACE_SKILLS },
    live: true,
    fetchedAt: '2026-09-10T00:00:00Z',
  });
  const user = userEvent.setup();
  render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);
  await screen.findByText('R Scripting');
  return { user, searchBox: screen.getByPlaceholderText(/Search skills/) };
}

/// Finding F5, in the desktop modal. The model-facing search (#242) and this one
/// were the same defect in two languages: the WHOLE query had to occur inside a
/// single field, so every word of `R scripting ggplot visualization` finds a
/// skill on its own and the phrase found none.
describe('BrowseSkillsModal — a multi-word search (finding F5)', () => {
  it('shows the union of what each word finds, best match first', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, 'R scripting ggplot visualization');

    expect(shownSkillNames()).toEqual([
      'ggplot2 Visualization',
      'R Scripting',
      'Data Visualization',
      'Python Scripting',
      'Clinical Biostatistics',
    ]);
  });

  /// The two single-word controls the QA run measured beside the phrase.
  it('finds exactly the two ggplot skills for `ggplot`', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, 'ggplot');

    expect(shownSkillNames()).toEqual(['ggplot2 Visualization', 'Data Visualization']);
  });

  it('puts the skill a query names first', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, 'r-scripting');

    expect(shownSkillNames()[0]).toBe('R Scripting');
  });
});

/** The section headings above the list, top to bottom. */
function shownHeadings(): string[] {
  return screen.queryAllByRole('heading', { level: 3 }).map((heading) => heading.textContent ?? '');
}

describe('BrowseSkillsModal — browsing is grouped, a search is ranked', () => {
  it('groups the catalog under its category headings, in registry order', async () => {
    await openWithMarketplaceSkills();

    expect(shownHeadings()).toEqual(['Core skills (4)', 'Biomedical analysis (3)']);
    expect(shownSkillNames()).toEqual([
      'Scientific Visual Communication',
      'ggplot2 Visualization',
      'R Scripting',
      'Python Scripting',
      'Clinical Biostatistics',
      'Data Visualization',
      'Single-cell',
    ]);
  });

  /// Under the category headings, Python Scripting (one term matched) would sit
  /// above Data Visualization (two) only because Core is listed before
  /// Biomedical — the ranking would be computed and then not shown.
  it('shows a search as one list in rank order', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, 'R scripting ggplot visualization');

    expect(shownHeadings()).toEqual(['Matches (5)']);
  });

  it('treats a query of only spaces as browsing', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, '   ');

    expect(shownHeadings()).toEqual(['Core skills (4)', 'Biomedical analysis (3)']);
  });

  it('keeps the category filter under a search', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.click(screen.getByRole('button', { name: 'Biomedical analysis' }));
    await user.type(searchBox, 'R scripting ggplot visualization');

    expect(shownSkillNames()).toEqual(['Data Visualization', 'Clinical Biostatistics']);
  });

  it('says so when nothing matches', async () => {
    const { user, searchBox } = await openWithMarketplaceSkills();

    await user.type(searchBox, 'xylophone');

    expect(shownSkillNames()).toEqual([]);
    expect(shownHeadings()).toEqual([]);
    expect(screen.getByText('No skills match your search.')).toBeInTheDocument();
  });
});

/** The checkbox on the row whose name is `name`. */
function rowCheckbox(name: string): HTMLElement {
  const label = screen.getByText(name, { selector: 'span' }).closest('label');
  if (!label) throw new Error(`no row named ${name}`);
  return within(label as HTMLElement).getByRole('checkbox');
}

/** The dialog's install button, whatever it currently says. */
function installButton(): HTMLElement {
  return screen.getByRole('button', { name: /^Install/ });
}

/** A successful install as the daemon reports it: one unit per package or skill. */
function landed(
  row: Pick<RegistrySkill, 'name'>,
  unit: { name: string; kind: 'single' | 'bundle'; skills: string[]; replaced?: boolean }
): InstallResult {
  return { ok: true, name: row.name, installed: [{ replaced: false, ...unit }] };
}

const componentNames = (prefix: string, count: number) =>
  Array.from({ length: count }, (_, i) => `${prefix}-${i + 1}`);

/// Item 5 of the 1.90.4 hold. A row is either one skill or a package of several,
/// and the copy counted ROWS as skills: selecting the 3-skill `primer-design`
/// package read "Install 1 skill" and then toasted "1 skill installed"; a
/// package plus one skill said "2 skills installed" for 15.
describe('BrowseSkillsModal — install copy counts skills, not rows', () => {
  it('prices a package at the skills its row says it holds', async () => {
    const { user } = await openWithMarketplaceSkills();

    await user.click(rowCheckbox('Single-cell'));

    // The row reads "14 skills · auto-applied"; the button must agree with it.
    expect(installButton().textContent).toBe('Install 14 skills');
    // The footer still counts what was checked.
    expect(screen.getByText('1 selected')).toBeInTheDocument();
  });

  it('adds a package and a single skill up to what will land', async () => {
    const { user } = await openWithMarketplaceSkills();

    await user.click(rowCheckbox('Single-cell'));
    await user.click(rowCheckbox('R Scripting'));

    expect(installButton().textContent).toBe('Install 15 skills');
    expect(screen.getByText('2 selected')).toBeInTheDocument();
  });

  /// `Select all (N)` counts rows, and that is right: it checks N boxes. What the
  /// selection installs is the button's job, and the two must not be confused.
  it('keeps Select all counting rows while the button counts skills', async () => {
    const { user } = await openWithMarketplaceSkills();

    await user.click(screen.getByRole('button', { name: 'Select all (7)' }));

    expect(screen.getByText('7 selected')).toBeInTheDocument();
    // 4 single skills + 6 + 13 + 14.
    expect(installButton().textContent).toBe('Install 37 skills');
  });

  it("toasts the daemon's count of what landed, naming the package", async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (row) =>
      row.id === 'single-cell'
        ? landed(row, { name: 'single-cell', kind: 'bundle', skills: componentNames('sc', 14) })
        : landed(row, { name: 'r-scripting', kind: 'single', skills: ['r-scripting'] })
    );
    const onClose = vi.fn();
    mocks.loadRegistry.mockResolvedValue({
      registry: { version: 2, source: 'test', extensions: [], skills: MARKETPLACE_SKILLS },
      live: true,
      fetchedAt: '2026-09-10T00:00:00Z',
    });
    const user = userEvent.setup();
    render(<BrowseSkillsModal onClose={onClose} onInstalled={vi.fn()} installedIds={new Set()} />);
    await screen.findByText('R Scripting');

    await user.click(rowCheckbox('Single-cell'));
    await user.click(rowCheckbox('R Scripting'));
    await user.click(installButton());

    expect(toastSuccess).toHaveBeenCalledTimes(1);
    // Installs run in registry order, and the toast names them in that order.
    expect(toastSuccess).toHaveBeenCalledWith({
      title: '15 skills installed',
      msg: 'Added to Biorouter Skills: r-scripting and single-cell (14 skills)',
    });
    expect(toastError).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  /// The toast reports what the daemon installed even where the catalogue's
  /// phrase is stale — the button can only repeat the row, the toast need not.
  it('trusts the daemon over the row when the two disagree', async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (row) =>
      landed(row, { name: 'single-cell', kind: 'bundle', skills: componentNames('sc', 12) })
    );
    const { user } = await openWithMarketplaceSkills();

    await user.click(rowCheckbox('Single-cell'));
    await user.click(installButton());

    expect(toastSuccess).toHaveBeenCalledWith({
      title: '12 skills installed',
      msg: 'Added to Biorouter Skills: single-cell (12 skills)',
    });
  });

  /// A partial failure. Nothing of a failed package lands — the importer stages
  /// it and swaps it in whole — so the failure names the row, and the rows that
  /// failed stay selected with the button re-priced to them.
  it('names a failed package, and leaves exactly it selected', async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (row) =>
      row.id === 'single-cell'
        ? { ok: false, name: row.name, error: 'network down' }
        : landed(row, { name: 'r-scripting', kind: 'single', skills: ['r-scripting'] })
    );
    const { user } = await openWithMarketplaceSkills();

    await user.click(rowCheckbox('Single-cell'));
    await user.click(rowCheckbox('R Scripting'));
    await user.click(installButton());

    expect(toastSuccess).toHaveBeenCalledWith({
      title: '1 skill installed',
      msg: 'Added to Biorouter Skills: r-scripting',
    });
    expect(toastError).toHaveBeenCalledWith({
      title: 'Single-cell was not installed',
      msg: 'network down',
    });
    expect(screen.getByText('1 selected')).toBeInTheDocument();
    expect(installButton().textContent).toBe('Install 14 skills');
  });

  it('counts failed selections, not their skills, when several fail', async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (row) => ({
      ok: false,
      name: row.name,
      error: 'disk full',
    }));
    const { user } = await openWithMarketplaceSkills();

    await user.click(rowCheckbox('Single-cell'));
    await user.click(rowCheckbox('Data Visualization'));
    await user.click(installButton());

    expect(toastSuccess).not.toHaveBeenCalled();
    expect(toastError).toHaveBeenCalledWith({
      title: '2 selections were not installed',
      msg: 'Data Visualization: disk full (and 1 more)',
    });
    expect(screen.getByText('2 selected')).toBeInTheDocument();
    expect(installButton().textContent).toBe('Install 27 skills');
  });

  /// The old re-selection matched each failure's TEXT against a row's name as a
  /// prefix, so a failed "Alignment Files" re-selected the "Alignment" that had
  /// just installed.
  it('re-selects a failed row by id, not by a name another row starts with', async () => {
    const row = (id: string, name: string): RegistrySkill => ({
      ...MARKETPLACE_SKILLS[6],
      id,
      name,
      type: '7 skills · auto-applied',
    });
    mocks.loadRegistry.mockResolvedValue({
      registry: {
        version: 2,
        source: 'test',
        extensions: [],
        skills: [row('alignment', 'Alignment'), row('alignment-files', 'Alignment Files')],
      },
      live: true,
      fetchedAt: '2026-09-10T00:00:00Z',
    });
    vi.mocked(installRegistrySkill).mockImplementation(async (skill) =>
      skill.id === 'alignment-files'
        ? { ok: false, name: skill.name, error: 'network down' }
        : landed(skill, { name: 'alignment', kind: 'bundle', skills: componentNames('al', 7) })
    );
    const user = userEvent.setup();
    render(<BrowseSkillsModal onClose={vi.fn()} onInstalled={vi.fn()} installedIds={new Set()} />);
    await screen.findByText('Alignment Files');

    await user.click(rowCheckbox('Alignment'));
    await user.click(rowCheckbox('Alignment Files'));
    await user.click(installButton());

    expect(screen.getByText('1 selected')).toBeInTheDocument();
    expect(rowCheckbox('Alignment Files')).toBeChecked();
    expect(rowCheckbox('Alignment')).not.toBeChecked();
  });
});

/** Open the dialog over `rows`, with `installedIds` as SkillsView would pass it. */
async function openOver(
  rows: RegistrySkill[],
  props: { installedIds?: Set<string>; onClose?: () => void } = {}
) {
  mocks.loadRegistry.mockResolvedValue({
    registry: { version: 2, source: 'test', extensions: [], skills: rows },
    live: true,
    fetchedAt: '2026-09-10T00:00:00Z',
  });
  const user = userEvent.setup();
  const onClose = props.onClose ?? vi.fn();
  const view = render(
    <BrowseSkillsModal
      onClose={onClose}
      onInstalled={vi.fn()}
      installedIds={props.installedIds ?? new Set()}
    />
  );
  await screen.findByText(rows[0].name);
  const rerenderWith = (installedIds: Set<string>) =>
    view.rerender(
      <BrowseSkillsModal onClose={onClose} onInstalled={vi.fn()} installedIds={installedIds} />
    );
  return { user, onClose, rerenderWith };
}

const alignmentRows = (): RegistrySkill[] =>
  [
    ['alignment', 'Alignment', 7],
    ['alignment-files', 'Alignment Files', 10],
    ['read-qc', 'Read QC', 7],
  ].map(([id, name, count]) => ({
    ...MARKETPLACE_SKILLS[6],
    id: id as string,
    name: name as string,
    type: `${count} skills · auto-applied`,
  }));

/// The tester's retry: "Alignment Files was not installed" stayed on screen
/// beside "10 skills installed | … alignment-files (10 skills)". Errors do not
/// expire by design, so whatever raised the report has to take it back.
describe('BrowseSkillsModal — a failure report is retracted when it stops being true', () => {
  it('dismisses the report when a retry of the failed row lands', async () => {
    let attempt = 0;
    vi.mocked(installRegistrySkill).mockImplementation(async (skill) =>
      skill.id === 'alignment-files' && attempt++ === 0
        ? { ok: false, name: skill.name, error: 'network down' }
        : landed(skill, { name: skill.id, kind: 'bundle', skills: componentNames(skill.id, 10) })
    );
    const { user, onClose } = await openOver(alignmentRows());

    await user.click(rowCheckbox('Alignment Files'));
    await user.click(installButton());
    expect(mocks.toastError).toHaveBeenCalledTimes(1);
    expect(mocks.activeToasts.size).toBe(1);

    await user.click(installButton());

    expect(toastSuccess).toHaveBeenLastCalledWith({
      title: '10 skills installed',
      msg: 'Added to Biorouter Skills: alignment-files (10 skills)',
    });
    expect(mocks.toastError).toHaveBeenCalledTimes(1);
    expect(mocks.activeToasts.size).toBe(0);
    expect(onClose).toHaveBeenCalled();
  });

  /// "3 selections were not installed" stayed up after two of the three landed.
  it('replaces a report of three with one naming only what is still missing', async () => {
    let round = 0;
    vi.mocked(installRegistrySkill).mockImplementation(async (skill) =>
      round === 0 || skill.id === 'read-qc'
        ? { ok: false, name: skill.name, error: 'disk full' }
        : landed(skill, { name: skill.id, kind: 'bundle', skills: componentNames(skill.id, 7) })
    );
    const { user } = await openOver(alignmentRows());

    await user.click(screen.getByRole('button', { name: 'Select all (3)' }));
    await user.click(installButton());
    expect([...mocks.activeToasts]).toEqual([
      'error:3 selections were not installed:Alignment: disk full (and 2 more)',
    ]);

    round = 1;
    await user.click(installButton());

    expect([...mocks.activeToasts]).toEqual(['error:Read QC was not installed:disk full']);
  });

  it('leaves a report alone when a run retried none of its rows', async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (skill) =>
      skill.id === 'alignment'
        ? { ok: false, name: skill.name, error: 'network down' }
        : landed(skill, { name: skill.id, kind: 'bundle', skills: componentNames(skill.id, 7) })
    );
    const { user } = await openOver(alignmentRows());

    await user.click(rowCheckbox('Alignment'));
    await user.click(installButton());
    await user.click(rowCheckbox('Alignment'));
    await user.click(rowCheckbox('Read QC'));
    await user.click(installButton());

    expect(mocks.dismiss).not.toHaveBeenCalled();
    expect([...mocks.activeToasts]).toEqual(['error:Alignment was not installed:network down']);
  });
});

/// Two windows. Window B installed a package while window A's dialog had it
/// selected; A kept offering it, installed it again over B's, and toasted the
/// overwrite as "12 skills installed". The catalog event now reaches A (see
/// `routes/skills.rs`), and what is left for this dialog is to believe it — and
/// to call an overwrite that still slips through what it is.
describe('BrowseSkillsModal — what another window installed', () => {
  it('stops counting a selected row as selected once it is installed', async () => {
    const { user, rerenderWith } = await openOver(alignmentRows());

    await user.click(rowCheckbox('Alignment Files'));
    expect(screen.getByText('1 selected')).toBeInTheDocument();

    rerenderWith(new Set(['alignment-files']));

    expect(screen.getByText('0 selected')).toBeInTheDocument();
    expect(rowCheckbox('Alignment Files')).not.toBeChecked();
    expect(rowCheckbox('Alignment Files')).toBeDisabled();
    expect(installButton().textContent).toBe('Install skills');
    expect(installButton()).toBeDisabled();
  });

  it('calls an install that replaced one already there a reinstall', async () => {
    vi.mocked(installRegistrySkill).mockImplementation(async (skill) =>
      landed(skill, {
        name: 'clinical-biostatistics',
        kind: 'bundle',
        skills: componentNames('cb', 12),
        replaced: true,
      })
    );
    const { user } = await openOver(alignmentRows());

    await user.click(rowCheckbox('Alignment'));
    await user.click(installButton());

    expect(toastSuccess).toHaveBeenCalledWith({
      title: '12 skills reinstalled',
      msg: 'Replaced in Biorouter Skills: clinical-biostatistics (12 skills)',
    });
  });
});
