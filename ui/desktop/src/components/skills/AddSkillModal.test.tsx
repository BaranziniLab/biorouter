import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import AddSkillModal from './AddSkillModal';
import { ADD_SKILL_COPY } from './copy';
import type { ImportPreview } from '../../api';

const mocks = vi.hoisted(() => ({
  previewSkillPackage: vi.fn(),
  installSkillPackage: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  getPathForFile: vi.fn(() => '/tmp/pack.zip'),
}));

vi.mock('../../api', () => ({
  previewSkillPackage: (...args: unknown[]) => mocks.previewSkillPackage(...args),
  installSkillPackage: (...args: unknown[]) => mocks.installSkillPackage(...args),
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

function preview(overrides: Partial<ImportPreview> = {}): ImportPreview {
  return {
    kind: 'bundle',
    id: 'hyperframes',
    displayName: 'HyperFrames',
    version: '0.8.12',
    entryPoint: 'hyperframes',
    groups: {},
    components: [
      {
        name: 'hyperframes',
        description: 'Router',
        directory: 'hyperframes',
        group: 'core',
        entryPoint: true,
      },
      {
        name: 'media-use',
        description: 'Media',
        directory: 'media-use',
        group: null,
        entryPoint: false,
      },
    ],
    evidence: 'codexPlugin',
    ambiguity: null,
    source: { url: null, reference: null, resolvedCommit: null, installer: null },
    shadows: [],
    fileCount: 12,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  // @ts-expect-error test shim
  window.electron = { getPathForFile: mocks.getPathForFile };
});

describe('AddSkillModal', () => {
  /**
   * The gap #115 names first: there was no repository import surface at all,
   * so a pasted URL had to go to the agent, which improvised with shell.
   */
  it('takes a repository URL and shows the preview the daemon returns', async () => {
    mocks.previewSkillPackage.mockResolvedValue({
      data: { status: 'needsChoice', planId: 'plan-1', preview: preview() },
    });
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText('https://github.com/owner/repo'), {
      target: { value: 'https://github.com/heygen-com/hyperframes' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));

    await waitFor(() => expect(mocks.previewSkillPackage).toHaveBeenCalledTimes(1));
    expect(mocks.previewSkillPackage.mock.calls[0][0].body).toEqual({
      url: 'https://github.com/heygen-com/hyperframes',
    });
    expect(await screen.findByText('HyperFrames')).toBeInTheDocument();
    expect(screen.getByText(ADD_SKILL_COPY.entryPoint('hyperframes'))).toBeInTheDocument();
    // The components keep their declared names, prefix or no prefix.
    expect(screen.getByText(/media-use/)).toBeInTheDocument();
  });

  /**
   * Installing by `planId` is what makes the preview binding: it installs the
   * archive that was previewed, not whatever the branch points at now.
   */
  it('installs the previewed plan rather than re-resolving the source', async () => {
    mocks.previewSkillPackage.mockResolvedValue({
      data: { status: 'needsChoice', planId: 'plan-7', preview: preview() },
    });
    mocks.installSkillPackage.mockResolvedValue({
      data: {
        status: 'installed',
        preview: preview(),
        installed: [
          {
            id: 'hyperframes',
            displayName: 'HyperFrames',
            kind: 'bundle',
            skills: ['hyperframes', 'media-use'],
            entryPoint: 'hyperframes',
            directory: '/skills/hyperframes',
            replaced: false,
            catalogGeneration: 4,
          },
        ],
      },
    });
    const onSaved = vi.fn();
    render(<AddSkillModal onClose={() => {}} onSaved={onSaved} />);

    fireEvent.change(screen.getByPlaceholderText('https://github.com/owner/repo'), {
      target: { value: 'https://github.com/heygen-com/hyperframes' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));
    await screen.findByText('HyperFrames');

    fireEvent.click(screen.getByRole('button', { name: 'Install 2 skills' }));
    await waitFor(() => expect(mocks.installSkillPackage).toHaveBeenCalledTimes(1));
    expect(mocks.installSkillPackage.mock.calls[0][0].body).toEqual({ planId: 'plan-7' });
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(mocks.toastSuccess.mock.calls[0][0].msg).toContain('2 skills');
  });

  /**
   * An ambiguous source offers both answers and takes neither by default.
   */
  it('offers a choice when the daemon cannot tell, and sends the answer', async () => {
    mocks.previewSkillPackage.mockResolvedValue({
      data: {
        status: 'needsChoice',
        planId: 'plan-9',
        preview: preview({
          displayName: 'mixed-bag',
          entryPoint: null,
          ambiguity: {
            reason: 'This source holds 2 skills side by side and declares no package manifest.',
            components: ['alpha', 'beta'],
          },
        }),
      },
    });
    mocks.installSkillPackage.mockResolvedValue({
      data: { status: 'installed', preview: preview(), installed: [] },
    });
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);

    fireEvent.change(screen.getByPlaceholderText('https://github.com/owner/repo'), {
      target: { value: 'https://github.com/o/r' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));

    expect(await screen.findByText(/declares no package manifest/)).toBeInTheDocument();
    // No plain "Install" button while the question is open.
    expect(screen.queryByRole('button', { name: /^Install \d/ })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Install as one bundle' }));
    await waitFor(() => expect(mocks.installSkillPackage).toHaveBeenCalledTimes(1));
    expect(mocks.installSkillPackage.mock.calls[0][0].body).toEqual({
      planId: 'plan-9',
      choice: 'bundle',
    });
  });

  it('reports a source it could not read, and installs nothing', async () => {
    mocks.previewSkillPackage.mockRejectedValue(new Error('example.com is not one of the hosts'));
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);
    fireEvent.change(screen.getByPlaceholderText('https://github.com/owner/repo'), {
      target: { value: 'https://example.com/x.zip' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Look up' }));

    expect(await screen.findByText(/not one of the hosts/)).toBeInTheDocument();
    expect(mocks.installSkillPackage).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('explains why a dropped file cannot be read in browser mode', async () => {
    mocks.getPathForFile.mockReturnValueOnce('');
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);

    const dropZone = screen.getByRole('button', { name: ADD_SKILL_COPY.dropZoneName });
    fireEvent.drop(dropZone, {
      dataTransfer: { files: [new File(['x'], 'pack.zip')] },
    });

    expect(await screen.findByText(/running on another machine/)).toBeInTheDocument();
    expect(mocks.previewSkillPackage).not.toHaveBeenCalled();
  });
});

describe('AddSkillModal: help on demand, not on the page', () => {
  it('keeps the repository explanation out of view and on the field as its description', () => {
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);

    expect(
      screen.getByRole('textbox', { name: ADD_SKILL_COPY.repositoryLabel })
    ).toHaveAccessibleDescription(ADD_SKILL_COPY.repositoryHelp);
    expect(
      screen.getByRole('button', { name: `About ${ADD_SKILL_COPY.repositoryLabel.toLowerCase()}` })
    ).toBeInTheDocument();
  });

  it('makes the drop zone one line that the keyboard can reach', () => {
    render(<AddSkillModal onClose={() => {}} onSaved={() => {}} />);

    const zone = screen.getByRole('button', { name: ADD_SKILL_COPY.dropZoneName });
    expect(zone).toHaveTextContent(ADD_SKILL_COPY.dropZone);
    expect(zone.textContent).toBe(ADD_SKILL_COPY.dropZone);
  });
});
