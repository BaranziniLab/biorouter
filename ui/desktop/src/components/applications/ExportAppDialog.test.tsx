import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { StrictMode, type ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import ApplicationsView from './ApplicationsView';
import ExportAppDialog from './ExportAppDialog';
import type { AppManifest } from './appManagement';
import type { AppExportRequest, AppExportResult } from '../../utils/appExportRequest';
import { toastError, toastSuccess, toastWarning } from '../../toasts';
import { appsCopy } from './copy';

const copy = appsCopy.exportDialog;

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../../toasts', () => ({
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  toastWarning: vi.fn(),
}));

/**
 * An app that reaches knowledge bases three ways, as the manifest allows: its agent's own, a
 * `knowledge` data source, and its memory. The dialog used to list only the first, while a full
 * export carried all of them. It also names a skill Biorouter ships, which no export carries.
 */
const app = {
  id: 'cohort-explorer',
  title: 'Cohort Explorer',
  kind: 'agentic' as const,
  created_at: 1750000000,
  updated_at: 1750000000,
  agent: {
    knowledge_base: 'ms-cohort',
    skills: [
      'ggplot-visualization',
      'office-word',
      'Office-Word',
      'knowledge-bases',
      'Knowledge-Bases',
    ],
    extensions: ['developer', 'spokeagent', 'autovisualiser'],
    capabilities: {
      data: { sources: [{ name: 'trials', kind: 'knowledge', ids: ['trials-kb'] }] },
      memory: { kb: 'app-memory', shared_kb: 'lab-memory' },
    },
  },
} as unknown as AppManifest;

const exported = (target = '/Users/me/Desktop/cohort-explorer'): AppExportResult => ({
  status: 'exported',
  target,
  legacy: false,
  summary: {
    mode: 'launcher',
    files: 12,
    kbs: [],
    skills: [],
    extensions: [],
    notes: [],
    lintWarnings: [],
    skipped: [],
  },
});

function renderDialog(
  onExport = vi.fn(async () => exported()),
  browser = false,
  { strict = false }: { strict?: boolean } = {}
) {
  const onClose = vi.fn();
  const user = userEvent.setup();
  const dialog = (
    <ExportAppDialog app={app} onExport={onExport} onClose={onClose} browser={browser} />
  );
  render(strict ? <StrictMode>{dialog}</StrictMode> : dialog);
  return { user, onExport, onClose };
}

const requestOf = (onExport: ReturnType<typeof vi.fn>, call = 0) =>
  onExport.mock.calls[call][0] as AppExportRequest;

describe('ExportAppDialog', () => {
  it('starts on launcher mode and sends no selection for it', async () => {
    const { user, onExport, onClose } = renderDialog();
    expect(screen.getByRole('radio', { name: /^Launcher/ })).toBeChecked();
    // The safety fact stays visible, not behind help.
    expect(screen.getByText(copy.credentials)).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: copy.export }));

    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(requestOf(onExport)).toEqual({
      mode: 'launcher',
      bundleDaemon: false,
      allowLintErrors: false,
    });
  });

  it('lists every knowledge base the app reaches and nothing that ships with Biorouter', async () => {
    const { user } = renderDialog();
    await user.click(screen.getByRole('radio', { name: /^Full/ }));

    const kbs = screen.getByRole('group', { name: copy.knowledgeBases });
    for (const kb of ['app-memory', 'lab-memory', 'ms-cohort', 'trials-kb']) {
      expect(within(kbs).getByRole('checkbox', { name: kb })).toBeChecked();
    }
    const skills = screen.getByRole('group', { name: copy.skills });
    expect(within(skills).getByRole('checkbox', { name: 'ggplot-visualization' })).toBeChecked();
    // The recipient's install seeds its own; one installed over it could not be removed.
    expect(screen.queryByRole('checkbox', { name: /office-word/i })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: /knowledge-bases/i })).toBeNull();
    const extensions = screen.getByRole('group', { name: copy.extensions });
    expect(within(extensions).getByRole('checkbox', { name: 'spokeagent' })).toBeChecked();
    expect(screen.queryByRole('checkbox', { name: 'developer' })).toBeNull();
    expect(screen.queryByRole('checkbox', { name: 'autovisualiser' })).toBeNull();
    expect(screen.getByRole('checkbox', { name: copy.includeBiorouter })).not.toBeChecked();
  });

  it('sends all three lists in full mode, an empty one meaning none', async () => {
    const { user, onExport } = renderDialog();
    await user.click(screen.getByRole('radio', { name: /^Full/ }));
    await user.click(screen.getByRole('checkbox', { name: 'ms-cohort' }));
    await user.click(screen.getByRole('checkbox', { name: 'ggplot-visualization' }));
    await user.click(screen.getByRole('checkbox', { name: 'spokeagent' }));
    await user.click(screen.getByRole('checkbox', { name: copy.includeBiorouter }));
    await user.click(screen.getByRole('button', { name: copy.export }));

    await waitFor(() => expect(onExport).toHaveBeenCalled());
    expect(requestOf(onExport)).toEqual({
      mode: 'full',
      bundleDaemon: true,
      allowLintErrors: false,
      include: {
        knowledge_bases: ['app-memory', 'lab-memory', 'trials-kb'],
        skills: [],
        extensions: [],
      },
    });
  });

  it('shows the harness findings and exports anyway only when asked, into the same folder', async () => {
    const onExport = vi
      .fn()
      .mockResolvedValueOnce({
        status: 'lint',
        message: 'harness ERRORs block export',
        findings: ['index.html: #missing is not in the page', 'inline script is blocked'],
        retry: 'token-1',
      })
      .mockResolvedValueOnce(exported());
    const { user, onClose } = renderDialog(onExport);
    await user.click(screen.getByRole('button', { name: copy.export }));

    const findings = await screen.findByRole('list', { name: copy.problemsFound });
    expect(within(findings).getByText(/#missing/)).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    await user.click(screen.getByRole('button', { name: copy.exportAnyway }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(requestOf(onExport, 1)).toEqual({
      mode: 'launcher',
      bundleDaemon: false,
      allowLintErrors: true,
      retry: 'token-1',
    });
  });

  describe('the app check’s face', () => {
    const findings = Array.from(
      { length: 11 },
      (_, n) => `finding ${n + 1}: #part-${n} is not in the page`
    );
    const lintAnswer = (list = findings): AppExportResult => ({
      status: 'lint',
      message: 'harness ERRORs block export',
      findings: list,
      retry: 'token-1',
    });

    it('lists every finding in one warning Note, and nothing scrolls inside the body', async () => {
      const { user } = renderDialog(vi.fn(async () => lintAnswer()));
      await user.click(screen.getByRole('button', { name: copy.export }));

      const list = await screen.findByRole('list', { name: copy.problemsFound });
      // All eleven, none dropped behind a count: the Note's own ceiling folds a long list.
      expect(within(list).getAllByRole('listitem')).toHaveLength(11);
      expect(within(list).getByText(findings[10])).toBeInTheDocument();
      const note = screen.getByTestId('export-lint-findings');
      expect(note).toContainElement(list);
      expect(note.className).toContain('bg-wash-warning');
      expect(list.className).not.toMatch(/overflow|max-h/);
    });

    it('keeps the consequence visible and puts the advice behind the title’s InfoTip', async () => {
      const { user } = renderDialog(vi.fn(async () => lintAnswer()));
      await user.click(screen.getByRole('button', { name: copy.export }));
      const dialog = await screen.findByRole('dialog', { name: copy.lintTitle(app.title) });

      expect(dialog).toHaveAccessibleDescription(copy.lintSubtitle);
      const tip = within(dialog).getByRole('button', { name: `About ${copy.problemsFound}` });
      expect(tip).toHaveAccessibleDescription(copy.lintHelp);
      // The advice's only node is the InfoTip's hidden description, not a paragraph.
      expect(within(dialog).getByText(copy.lintHelp)).toHaveAttribute(
        'data-slot',
        'info-tip-description'
      );
    });

    it('moves focus to the findings when they arrive', async () => {
      const { user } = renderDialog(vi.fn(async () => lintAnswer()));
      await user.click(screen.getByRole('button', { name: copy.export }));
      const list = await screen.findByRole('list', { name: copy.problemsFound });
      await waitFor(() => expect(list).toHaveFocus());
    });

    it('stays on the findings while Export anyway runs, and says it is working', async () => {
      let answer: (result: AppExportResult) => void = () => {};
      const onExport = vi
        .fn()
        .mockResolvedValueOnce(lintAnswer(['inline script is blocked']))
        .mockImplementationOnce(
          () => new Promise<AppExportResult>((resolve) => (answer = resolve))
        );
      const { user, onClose } = renderDialog(onExport);
      await user.click(screen.getByRole('button', { name: copy.export }));
      await user.click(await screen.findByRole('button', { name: copy.exportAnyway }));

      // Still the findings, not the mode choice it started from.
      expect(screen.getByRole('list', { name: copy.problemsFound })).toBeInTheDocument();
      expect(screen.queryByRole('radiogroup', { name: copy.mode })).toBeNull();
      expect(screen.getByRole('button', { name: copy.exporting })).toBeDisabled();

      answer(exported());
      await waitFor(() => expect(onClose).toHaveBeenCalled());
    });
  });

  it('holds the mode choice while an export runs', async () => {
    let answer: (result: AppExportResult) => void = () => {};
    const { user, onClose } = renderDialog(
      vi.fn(() => new Promise<AppExportResult>((resolve) => (answer = resolve)))
    );
    await user.click(screen.getByRole('button', { name: copy.export }));
    expect(screen.getByRole('radio', { name: /^Launcher/ })).toBeDisabled();
    expect(screen.getByRole('radio', { name: /^Full/ })).toBeDisabled();
    expect(screen.getByRole('button', { name: copy.exporting })).toBeDisabled();
    answer(exported());
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('says an app with nothing to include in a neutral Note', async () => {
    const user = userEvent.setup();
    const bare = { ...app, agent: { model: { provider: 'p', model: 'm' } } } as AppManifest;
    render(<ExportAppDialog app={bare} onExport={vi.fn()} onClose={vi.fn()} />);
    await user.click(screen.getByRole('radio', { name: /^Full/ }));
    const note = screen.getByTestId('export-nothing-to-include');
    expect(note).toHaveTextContent(copy.nothingToInclude);
    expect(note.className).toContain('bg-background-fill');
  });

  it('cancel sends nothing', async () => {
    const { user, onExport, onClose } = renderDialog();
    await user.click(screen.getByRole('button', { name: copy.cancel }));
    expect(onClose).toHaveBeenCalled();
    expect(onExport).not.toHaveBeenCalled();
  });

  /**
   * The development renderer mounts under `React.StrictMode` (renderer.tsx), which runs every
   * effect's cleanup and then the effect again on mount. An "is it still mounted" ref that only
   * the cleanup ever wrote stayed false after that, so the dialog dropped the export's answer:
   * it sat on "Exporting…" after the folder was written, and never showed the app check's face.
   * The cases above render without StrictMode, which is why they passed through it.
   */
  describe('under React.StrictMode, as the development renderer mounts it', () => {
    it('closes once the export answers', async () => {
      const { user, onExport, onClose } = renderDialog(undefined, false, { strict: true });
      await user.click(screen.getByRole('button', { name: copy.export }));
      await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
      expect(onExport).toHaveBeenCalledTimes(1);
    });

    it('turns to the app check’s face, and Export anyway closes it', async () => {
      const onExport = vi
        .fn()
        .mockResolvedValueOnce({
          status: 'lint',
          message: 'harness ERRORs block export',
          findings: ['index.html: #missing is not in the page', 'inline script is blocked'],
          retry: 'token-1',
        })
        .mockResolvedValueOnce(exported());
      const { user, onClose } = renderDialog(onExport, false, { strict: true });
      await user.click(screen.getByRole('button', { name: copy.export }));

      const findings = await screen.findByRole('list', { name: copy.problemsFound });
      expect(within(findings).getAllByRole('listitem')).toHaveLength(2);
      await waitFor(() => expect(findings).toHaveFocus());
      expect(onClose).not.toHaveBeenCalled();

      await user.click(screen.getByRole('button', { name: copy.exportAnyway }));
      await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
      expect(requestOf(onExport, 1)).toEqual({
        mode: 'launcher',
        bundleDaemon: false,
        allowLintErrors: true,
        retry: 'token-1',
      });
    });

    it('closes when the export throws, rather than staying busy', async () => {
      // A failure the dialog does not keep: the view says why in a toast.
      const { user, onClose } = renderDialog(
        vi.fn(async () => {
          throw new Error('ipc gone');
        }),
        false,
        { strict: true }
      );
      await user.click(screen.getByRole('button', { name: copy.export }));
      await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    });
  });

  it('offers no daemon binary in a browser, which downloads a zip', async () => {
    const { user, onExport } = renderDialog(
      vi.fn(async () => exported('cohort-explorer.zip')),
      true
    );
    expect(screen.getByText(/Downloads the app as a \.zip/)).toBeInTheDocument();
    await user.click(screen.getByRole('radio', { name: /^Full/ }));
    expect(screen.queryByRole('checkbox', { name: copy.includeBiorouter })).toBeNull();
    await user.click(screen.getByRole('button', { name: copy.export }));
    await waitFor(() => expect(onExport).toHaveBeenCalled());
    expect(requestOf(onExport).bundleDaemon).toBe(false);
  });
});

describe('ApplicationsView export', () => {
  let exportApp: ReturnType<typeof vi.fn>;
  let directoryChooser: ReturnType<typeof vi.fn>;
  let writeFile: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          ({ ok: true, status: 200, json: async () => [app], text: async () => '' }) as Response
      )
    );
    exportApp = vi.fn(async () => exported());
    directoryChooser = vi.fn();
    writeFile = vi.fn();
    window.electron = {
      platform: 'darwin',
      getSecretKey: vi.fn().mockResolvedValue('test-secret'),
      getUserActionKey: vi.fn().mockResolvedValue('test-proof'),
      openExternal: vi.fn().mockResolvedValue(undefined),
      exportApp,
      directoryChooser,
      writeFile,
    } as unknown as typeof window.electron;
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  async function exportFromView() {
    const user = userEvent.setup();
    // Under StrictMode, as the development renderer mounts the view: the dialog closing below
    // is what a StrictMode remount used to break.
    render(
      <StrictMode>
        <MemoryRouter>
          <ApplicationsView />
        </MemoryRouter>
      </StrictMode>
    );
    // Radix opens a dropdown on pointerdown, not click.
    fireEvent.pointerDown(
      await screen.findByRole('button', { name: appsCopy.moreActionsNamed('Cohort Explorer') }),
      { button: 0, ctrlKey: false }
    );
    await user.click(await screen.findByRole('menuitem', { name: appsCopy.exportItem }));
    await user.click(await screen.findByRole('button', { name: copy.export }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  }

  it('hands the export to the main process, naming only the app and the choice', async () => {
    await exportFromView();
    expect(exportApp).toHaveBeenCalledWith('cohort-explorer', {
      mode: 'launcher',
      bundleDaemon: false,
      allowLintErrors: false,
    });
    // The renderer neither asks for a folder nor writes a file: main does both.
    expect(directoryChooser).not.toHaveBeenCalled();
    expect(writeFile).not.toHaveBeenCalled();
    expect(toastSuccess).toHaveBeenCalledWith({
      title: 'Cohort Explorer',
      msg: 'Exported to /Users/me/Desktop/cohort-explorer.',
    });
    expect(toastWarning).not.toHaveBeenCalled();
  });

  it('shows what the export left out, and why, apart from the success', async () => {
    const result = exported();
    if (result.status === 'exported') {
      result.summary.notes = ["skipped knowledge base 'ms-cohort': it is private"];
    }
    exportApp.mockResolvedValue(result);
    await exportFromView();
    // The daemon's clause, as a sentence: its own words, opened with a capital and stopped.
    expect(toastWarning).toHaveBeenCalledWith(
      expect.objectContaining({ msg: "Skipped knowledge base 'ms-cohort': it is private." })
    );
  });

  it('says each thing left out as its own sentence, unclamped, under the app’s name', async () => {
    const result = exported();
    if (result.status === 'exported') {
      result.summary.notes = [
        "skipped knowledge base 'ms-cohort': it is private",
        "skill 'ggplot-visualization' not installed; skipped",
        'The folder it replaced could not be moved to the Trash; it is at /tmp/x.',
      ];
    }
    exportApp.mockResolvedValue(result);
    await exportFromView();
    expect(toastWarning).toHaveBeenCalledWith({
      title: 'Cohort Explorer',
      msg:
        "Skipped knowledge base 'ms-cohort': it is private. " +
        "Skill 'ggplot-visualization' not installed; skipped. " +
        'The folder it replaced could not be moved to the Trash; it is at /tmp/x.',
      clampMessage: false,
    });
  });

  it('says why an export failed', async () => {
    exportApp.mockResolvedValue({
      status: 'failed',
      message: "There is no app 'cohort-explorer'.",
    });
    await exportFromView();
    expect(toastError).toHaveBeenCalledWith({
      title: 'Cohort Explorer',
      msg: "There is no app 'cohort-explorer'.",
    });
  });
});
