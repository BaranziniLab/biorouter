/**
 * An explicitly empty knowledge selection is captured as explicitly empty.
 *
 * The two states serialize differently and `workflow/runtime.rs` reads them
 * differently. An ABSENT `knowledge_bases` means "this workflow has nothing to
 * say", so every chat the workflow starts re-derives a selection from the
 * replaying machine and sees every base. A PRESENT-but-empty block means "select
 * none", and `apply_knowledge_selection` hides them all.
 *
 * Switching every base off produced the FIRST of those, so the one gesture that
 * says "no knowledge bases" was stored as the one that says "whatever you have".
 * `service.rs`'s own capture already got this right — `knowledge_capture_tests`
 * pins `an_empty_visible_set_is_captured_not_dropped` — and this modal was the
 * remaining half.
 *
 * ⚠ **Why a separate file from CreateWorkflowFromSessionModal.test.tsx.** That
 * suite's knowledge-base block is `describe('when its selection cannot be
 * read')`, whose whole fixture is a read that FAILS. What is under test here is
 * the opposite fixture — a read that SUCCEEDS and answers "nothing is selected"
 * — and the two cannot share a `beforeEach`. The distinction between them is the
 * point: a failed read must stay distinguishable from an empty selection, so
 * both are asserted here side by side.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import CreateWorkflowFromSessionModal from '../CreateWorkflowFromSessionModal';
import { createWorkflow, getActive, listBases } from '../../../api/sdk.gen';
import type { CreateWorkflowResponse } from '../../../api/types.gen';
import { saveWorkflow } from '../../../workflow/workflow_management';

vi.mock('../../../api/sdk.gen', () => ({
  createWorkflow: vi.fn(),
  getExtensions: vi.fn().mockResolvedValue({ data: { extensions: [] }, error: undefined }),
  getSessionExtensions: vi.fn().mockResolvedValue({ data: { extensions: [] }, error: undefined }),
  listBases: vi.fn(),
  getActive: vi.fn(),
  skillCatalogHandler: vi
    .fn()
    .mockResolvedValue({ data: { generation: 1, roots: [], skills: [], bundles: [] } }),
}));

vi.mock('../../../toasts', () => ({ toastError: vi.fn() }));

vi.mock('../../../workflow/workflow_management', () => ({
  saveWorkflow: vi.fn().mockResolvedValue('saved-workflow-id'),
}));

const mockCreateWorkflow = vi.mocked(createWorkflow);
const mockGetActive = vi.mocked(getActive);
const mockListBases = vi.mocked(listBases);
const mockSaveWorkflow = vi.mocked(saveWorkflow);

/** Two bases on the machine, so "nothing selected" is a choice and not a vacuum. */
const BASES = [
  { id: 'lab-notes', name: 'Lab notes' },
  { id: 'soul', name: 'Soul' },
];

describe('CreateWorkflowFromSessionModal — capturing the knowledge selection', () => {
  const props = {
    isOpen: true,
    onClose: vi.fn(),
    sessionId: 'test-session-id',
    onWorkflowCreated: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(window, {
      electron: { listSkillDirs: vi.fn().mockResolvedValue([]) },
    });
    // A generation that says nothing about knowledge bases, so what is captured
    // is this modal's own read and nothing else.
    const generated: CreateWorkflowResponse = {
      workflow: {
        title: 'Analyzed Workflow Title',
        description: 'Analyzed description',
        instructions: 'Analyzed instructions',
        activities: [],
        parameters: [],
      },
    } as unknown as CreateWorkflowResponse;
    mockCreateWorkflow.mockResolvedValue({
      data: generated,
      error: undefined,
      request: new globalThis.Request('http://localhost/test'),
      response: new globalThis.Response(),
    } as never);
    mockListBases.mockResolvedValue({ data: BASES, error: undefined } as never);
  });

  /** The selection the daemon answers with: everything in `hidden` is hidden. */
  function selectionAnswers(hidden: string[], primary: string | null = null) {
    mockGetActive.mockResolvedValue({
      data: { primary_kb: primary, hidden_kbs: hidden },
      error: undefined,
    } as never);
  }

  async function saveTheWorkflow(user: ReturnType<typeof userEvent.setup>) {
    await waitFor(
      () => {
        expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
      },
      { timeout: 3000 }
    );
    await user.click(screen.getByTestId('create-workflow-button'));
    await waitFor(() => {
      expect(mockSaveWorkflow).toHaveBeenCalled();
    });
    return mockSaveWorkflow.mock.calls[0]?.[0];
  }

  it('stores a chat that has every base switched off as an explicitly empty selection', async () => {
    // The read SUCCEEDS and says both bases are hidden. Under the old rule this
    // was indistinguishable from a failed read, and both wrote no key at all —
    // so the workflow re-derived a selection and saw every base.
    selectionAnswers(['lab-notes', 'soul']);
    const user = userEvent.setup();
    render(<CreateWorkflowFromSessionModal {...props} />);

    const saved = await saveTheWorkflow(user);

    expect(saved?.knowledge_bases).toEqual({ default: null, visible: [] });
  });

  /*
   * ⚠ **Not covered here: the same gesture made through the picker** — opening
   * Advanced options and switching the last base off. It reaches the identical
   * emit through `resourceEditsRef.current.knowledgeBases`, so the rule above is
   * what decides it either way; what is missing is the click path. Radix's
   * `Collapsible` around Advanced options would not open from a fresh spec file
   * in jsdom (the trigger's own label renders, its content never mounts), and
   * `WorkflowResourcePicker` belongs to another change in flight, so driving its
   * internals from here would collide rather than cover. Stated rather than left
   * as a gap someone has to rediscover.
   */

  it('stores a selection it could read, with its primary', async () => {
    // The ordinary case, here so the empty ones cannot pass by capturing
    // nothing at all.
    selectionAnswers([], 'soul');
    const user = userEvent.setup();
    render(<CreateWorkflowFromSessionModal {...props} />);

    const saved = await saveTheWorkflow(user);

    expect(saved?.knowledge_bases).toEqual({
      default: 'soul',
      visible: ['lab-notes', 'soul'],
    });
  });

  it('still captures nothing when the selection could not be read', async () => {
    // The distinction this must not erase. A failed read is not a statement
    // about the chat, so the workflow says nothing about knowledge bases and
    // each chat it starts derives its own.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    mockGetActive.mockResolvedValue({ data: undefined, error: { message: 'no proof' } } as never);
    const user = userEvent.setup();
    render(<CreateWorkflowFromSessionModal {...props} />);

    const saved = await saveTheWorkflow(user);

    expect(saved?.knowledge_bases).toBeUndefined();
    expect(warn).toHaveBeenCalledWith('Knowledge selection not read:', expect.any(String));
    warn.mockRestore();
  });

  it('captures nothing when the machine has no knowledge bases at all', async () => {
    // There was no selection to make, so there is no selection to state — which
    // is also what `knowledge_bases_for_session` answers for this case.
    mockListBases.mockResolvedValue({ data: [], error: undefined } as never);
    selectionAnswers([]);
    const user = userEvent.setup();
    render(<CreateWorkflowFromSessionModal {...props} />);

    const saved = await saveTheWorkflow(user);

    expect(saved?.knowledge_bases).toBeUndefined();
  });
});
