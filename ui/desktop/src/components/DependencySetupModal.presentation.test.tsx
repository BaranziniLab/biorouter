/**
 * The startup dependency prompt's presentation (first run, FR-S-7): one coral button per view,
 * help in tooltips and never in `title=`, glyphs from the icon registry, commands drawn as code,
 * and one sentence saying why. The real `ModalShell` renders here, so the subtitle is the dialog's
 * description and the footer is the real one.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, render, screen, within } from '@testing-library/react';
import type { DependencyEvent, DependencyInfo } from '../utils/dependencyChecker';
import DependencySetupModal from './DependencySetupModal';

type CliStatus = Awaited<ReturnType<typeof window.electron.cliStatus>>;

let emit: (e: DependencyEvent) => void = () => {};
let cliStatus: CliStatus | null = null;

function dep(name: string, overrides: Partial<DependencyInfo> = {}): DependencyInfo {
  return {
    name,
    displayName: name,
    version: null,
    installed: false,
    installCmd: `brew install ${name}`,
    requiresSudo: false,
    downloadUrl: `https://example.org/${name}`,
    required: true,
    ...overrides,
  };
}

beforeEach(() => {
  cliStatus = null;
  window.electron = {
    on: (_channel: string, handler: (e: unknown, ...args: unknown[]) => void) => {
      emit = (payload: DependencyEvent) => handler({}, payload);
      return () => {};
    },
    cliStatus: async () => cliStatus,
    installCli: vi.fn(),
    installDependency: vi.fn().mockResolvedValue({ started: true }),
    openExternal: vi.fn(),
    createChatWindow: vi.fn(),
    dependencyEnvironment: async () => ({}),
  } as unknown as typeof window.electron;
  sessionStorage.clear();
  localStorage.clear();
});

const isCoral = (button: HTMLElement) => button.classList.contains('bg-background-accent');
const coralButtons = () => screen.getAllByRole('button').filter(isCoral);

async function show(deps: DependencyInfo[]) {
  render(<DependencySetupModal />);
  await act(async () => emit({ type: 'check-results', deps }));
  return screen.findByRole('dialog');
}

describe('DependencySetupModal: one coral button (principle 7)', () => {
  it('makes "Install all" the one primary and every row Install secondary', async () => {
    await show([dep('node'), dep('uv')]);
    const all = screen.getByRole('button', { name: 'Install all (2)' });
    expect(coralButtons()).toEqual([all]);
    for (const row of screen.getAllByRole('button', { name: 'Install' })) {
      expect(row.className).toContain('br-fill-rest');
    }
  });

  it('keeps a lone Install coral when there is no batch', async () => {
    await show([dep('node')]);
    expect(coralButtons()).toEqual([screen.getByRole('button', { name: 'Install' })]);
  });

  it('keeps the lone real Install coral beside a row that has no installer', async () => {
    // Only one of the two has an automated installer, so there is no "Install all", and the row
    // without one offers no Install: Node.js's is the one install action on screen.
    await show([
      dep('node'),
      dep('rust', { installCmd: '# Install rust via your package manager' }),
    ]);
    expect(screen.queryByRole('button', { name: /install all/i })).toBeNull();
    expect(coralButtons()).toEqual([screen.getByRole('button', { name: 'Install' })]);
  });

  it('draws no coral when two single actions stand side by side with no batch', async () => {
    // One row and the CLI card: two install actions, no "Install all", so neither is the primary.
    cliStatus = {
      bundled: '/app/biorouter',
      onPath: true,
      pathLocation: '/usr/local/bin/biorouter',
      bundledVersion: '1.92.1',
      pathVersion: '1.90.4',
      needsUpdate: true,
      brokenOnPath: false,
    } as CliStatus;
    render(<DependencySetupModal />);
    await screen.findByRole('button', { name: 'Update' });
    await act(async () => emit({ type: 'check-results', deps: [dep('node')] }));
    expect(screen.getByRole('button', { name: 'Install' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install all/i })).toBeNull();
    expect(coralButtons()).toEqual([]);
  });

  it('ends the footer like every other dialog on this path: Dismiss, then the primary, both 32px', async () => {
    // FR-R2-2: the coral "Install all" sat before "Dismiss", and both were the 28px `sm` rung
    // while the Configure and privacy dialogs end Cancel, primary on the 32px default.
    await show([dep('node'), dep('uv')]);
    const dismiss = screen.getByRole('button', { name: 'Dismiss' });
    const all = screen.getByRole('button', { name: 'Install all (2)' });
    expect(dismiss.parentElement).toBe(all.parentElement);
    const footerButtons = Array.from(dismiss.parentElement!.querySelectorAll('button'));
    expect(footerButtons).toEqual([dismiss, all]);
    for (const button of footerButtons) {
      expect(button).toHaveClass('h-control-md');
      expect(button).not.toHaveClass('h-control-sm');
    }
  });

  it('counts the CLI card with the rows', async () => {
    cliStatus = {
      bundled: '/app/biorouter',
      onPath: true,
      pathLocation: '/usr/local/bin/biorouter',
      bundledVersion: '1.92.1',
      pathVersion: '1.90.4',
      needsUpdate: true,
      brokenOnPath: false,
    } as CliStatus;
    render(<DependencySetupModal />);
    const update = await screen.findByRole('button', { name: 'Update' });
    // The CLI card alone: its button is the view's primary.
    expect(coralButtons()).toEqual([update]);
    await act(async () => emit({ type: 'check-results', deps: [dep('node'), dep('uv')] }));
    // With a batch beside it, the batch is.
    expect(coralButtons()).toEqual([screen.getByRole('button', { name: 'Install all (2)' })]);
    expect(isCoral(update)).toBe(false);
  });
});

describe('DependencySetupModal: a row with no installer offers no Install', () => {
  /** The row (the panel holding the name) for one dependency. */
  const rowOf = (name: string) =>
    screen.getByText(name, { selector: 'p' }).closest('.br-fill-surface') as HTMLElement;

  it('prints a shell comment as a prose note, keeps Download, and has no Install', async () => {
    await show([
      dep('node'),
      dep('git', { installCmd: '# Install git via your system package manager' }),
    ]);
    const row = rowOf('git');
    expect(within(row).queryByRole('button', { name: 'Install' })).toBeNull();
    expect(within(row).getByRole('button', { name: 'Download' })).toBeInTheDocument();
    const note = within(row).getByText('Install git via your system package manager');
    expect(note.textContent).not.toContain('#');
    expect(note).not.toHaveClass('font-mono');
    // The row with a real command still prints it as code, with its Install.
    const node = rowOf('node');
    expect(within(node).getByText('brew install node')).toHaveClass('font-mono');
    expect(within(node).getByRole('button', { name: 'Install' })).toBeInTheDocument();
  });

  it('leaves Download as the only action, with no empty line, for an empty command and a page', async () => {
    await show([dep('node'), dep('llama-server', { installCmd: '' })]);
    const row = rowOf('llama-server');
    expect(within(row).queryByRole('button', { name: 'Install' })).toBeNull();
    expect(
      within(row)
        .getAllByRole('button')
        .map((b) => b.textContent)
    ).toEqual(['Download']);
    // The name is the row's one line: no empty paragraph under it.
    const lines = Array.from(row.querySelectorAll('p'));
    expect(lines.map((p) => p.textContent)).toEqual(['llama-server']);
  });

  it('says what to do when there is neither an installer nor a download page', async () => {
    await show([
      dep('node'),
      dep('python', { displayName: 'Python', installCmd: '', downloadUrl: '' }),
    ]);
    const row = rowOf('Python');
    expect(within(row).queryAllByRole('button')).toEqual([]);
    const note = within(row).getByText('Install Python via your system package manager');
    expect(note).not.toHaveClass('font-mono');
  });

  it('keeps Install on a failed row as the retry', async () => {
    await show([dep('node'), dep('uv')]);
    await act(async () => emit({ type: 'install-error', dep: 'uv', error: 'no network' }));
    expect(within(rowOf('uv')).getByRole('button', { name: 'Install' })).toBeInTheDocument();
  });
});

describe('DependencySetupModal: names, glyphs and words', () => {
  it('keeps each button named by its visible label, with the help in a tooltip, not a title', async () => {
    await show([dep('node')]);
    const download = screen.getByRole('button', { name: 'Download' });
    expect(download).not.toHaveAttribute('title');
    const dialog = screen.getByRole('dialog');
    expect(dialog.querySelectorAll('[title]')).toHaveLength(0);

    await act(async () => emit({ type: 'install-error', dep: 'node', error: 'no network' }));
    const debug = screen.getByRole('button', { name: 'Debug with Biorouter' });
    expect(debug).not.toHaveAttribute('title');
  });

  it('marks a finished or failed row with registry glyphs, named, and no Unicode marks', async () => {
    await show([dep('node'), dep('uv')]);
    await act(async () => emit({ type: 'install-done', dep: 'node', installed: true }));
    await act(async () => emit({ type: 'install-error', dep: 'uv', error: 'no network' }));
    const dialog = screen.getByRole('dialog');
    expect(within(dialog).getByRole('img', { name: 'Installed' }).tagName.toLowerCase()).toBe(
      'svg'
    );
    expect(within(dialog).getByRole('img', { name: 'Failed' }).tagName.toLowerCase()).toBe('svg');
    expect(dialog.textContent).not.toMatch(/[✓✗→]/);
  });

  it('gives row titles the label role with no extra weight', async () => {
    await show([dep('node')]);
    const title = screen.getByText('node', { selector: 'p' });
    expect(title).toHaveClass('text-label');
    expect(title).not.toHaveClass('font-semibold');
  });

  it('says why once: the subtitle is the description and the footer does not repeat it', async () => {
    const dialog = await show([dep('node')]);
    expect(dialog).toHaveAccessibleDescription(
      'The following tools are required for Biorouter features. Install them to continue.'
    );
    expect(screen.queryByText(/may be limited until these are installed/)).toBeNull();
  });

  it('draws a command as code instead of printing Markdown backticks', async () => {
    cliStatus = {
      bundled: '/app/biorouter',
      onPath: true,
      pathLocation: '/usr/local/bin/biorouter',
      bundledVersion: '1.92.1',
      pathVersion: '1.90.4',
      needsUpdate: false,
      brokenOnPath: true,
    } as CliStatus;
    render(<DependencySetupModal />);
    const dialog = await screen.findByRole('dialog');
    expect(dialog.textContent).not.toContain('`');
    const code = Array.from(dialog.querySelectorAll('code')).map((c) => c.textContent);
    expect(code).toContain('biorouter');
    expect(dialog).toHaveAccessibleDescription(
      'The biorouter on your PATH no longer runs. Reinstall it from this app.'
    );
  });

  it('draws the version step with the registry arrow, read as "to"', async () => {
    cliStatus = {
      bundled: '/app/biorouter',
      onPath: true,
      pathLocation: '/usr/local/bin/biorouter',
      bundledVersion: '1.92.1',
      pathVersion: '1.90.4',
      needsUpdate: true,
      brokenOnPath: false,
    } as CliStatus;
    render(<DependencySetupModal />);
    const dialog = await screen.findByRole('dialog');
    const arrow = within(dialog).getByRole('img', { name: 'to' });
    expect(arrow.tagName.toLowerCase()).toBe('svg');
    expect(arrow.parentElement).toHaveTextContent(/Update 1\.90\.4\s*1\.92\.1 to match this app\./);
  });
});
