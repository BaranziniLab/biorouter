import { describe, it, expect, vi, beforeEach, afterEach, type MockInstance } from 'vitest';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import CreateWorkflowFromSessionModal from '../CreateWorkflowFromSessionModal';
import { createWorkflow, getActive, listBases, skillCatalogHandler } from '../../../api/sdk.gen';
import type { CreateWorkflowResponse, WorkflowKnowledgeBases } from '../../../api/types.gen';
import { saveWorkflow } from '../../../workflow/workflow_management';
import { reachGatedGetActive, USER_ACTION_KEY } from '../../../test/reachGate';

/** A promise the test settles by hand, so "which answer landed first" is a fact, not a race. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

vi.mock('../../../api/sdk.gen', () => ({
  createWorkflow: vi.fn(),
  // The modal and its child WorkflowFormFields now load extensions, knowledge
  // bases, and the active KB on open. Provide blank-but-correctly-shaped
  // resolutions so those effects don't throw.
  getExtensions: vi.fn().mockResolvedValue({
    data: { extensions: [] },
    error: undefined,
  }),
  getSessionExtensions: vi.fn().mockResolvedValue({
    data: { extensions: [] },
    error: undefined,
  }),
  listBases: vi.fn().mockResolvedValue({
    data: [],
    error: undefined,
  }),
  getActive: vi.fn().mockResolvedValue({
    data: { active_kb: null, hidden_kbs: [] },
    error: undefined,
  }),
  // ⚠ The skill picker reads the daemon's catalog now, not a renderer-side
  // filesystem scan (#113). Omitting this did not fail anything — the modal
  // catches the load and falls back to an empty list — so the test went on
  // passing while silently exercising a workflow with NO skills to attach.
  // A blank-but-correctly-shaped resolution is what the comment above promises.
  skillCatalogHandler: vi.fn().mockResolvedValue({
    data: {
      generation: 1,
      roots: [],
      skills: [
        {
          name: 'literature-review',
          description: 'Survey the literature',
          slug: 'literature-review',
          directory: '/skills/literature-review',
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
        },
      ],
      bundles: [],
    },
    error: undefined,
  }),
}));

vi.mock('../../../toasts', () => ({
  toastError: vi.fn(),
}));

vi.mock('../../../workflow/workflow_management', () => ({
  saveWorkflow: vi.fn().mockResolvedValue('saved-workflow-id'),
}));

const mockCreateWorkflow = vi.mocked(createWorkflow);
const mockGetActive = vi.mocked(getActive);
const mockListBases = vi.mocked(listBases);
const mockSkillCatalog = vi.mocked(skillCatalogHandler);
const mockSaveWorkflow = vi.mocked(saveWorkflow);

/** Cross a macrotask boundary, so every `.then` already queued has run. */
async function settleReads() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

/** Wait for the form, press "Create workflow", and return what was saved. */
async function saveTheWorkflow(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(
    () => {
      expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
    },
    { timeout: 2000 }
  );
  await user.click(screen.getByTestId('create-workflow-button'));
  await waitFor(() => {
    expect(mockSaveWorkflow).toHaveBeenCalled();
  });
  return mockSaveWorkflow.mock.calls[0]?.[0];
}

describe('CreateWorkflowFromSessionModal', () => {
  const defaultProps = {
    isOpen: true,
    onClose: vi.fn(),
    sessionId: 'test-session-id',
    onWorkflowCreated: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(window, {
      electron: {
        listSkillDirs: vi.fn().mockResolvedValue([]),
      },
    });
    const mockResponse: CreateWorkflowResponse = {
      workflow: {
        title: 'Analyzed Workflow Title',
        description: 'Analyzed description',
        instructions: 'Analyzed instructions with {{param1}}',
        prompt: 'Analyzed prompt',
        activities: ['activity1', 'activity2'],
        parameters: [
          {
            key: 'param1',
            description: 'Auto-detected parameter',
            input_type: 'string',
            requirement: 'required',
          },
        ],
        response: {
          json_schema: { type: 'object' },
        },
        extensions: [
          {
            type: 'platform',
            name: 'skills',
            description: 'Load and use skills from relevant directories',
            bundled: true,
            available_tools: [],
          },
        ],
        knowledge_bases: {
          default: 'research-kb',
          visible: ['research-kb'],
        },
        skills: ['literature-review'],
        // The generator really does produce all three of these — settings comes
        // from Agent::create_workflow's provider/model pin, author from the
        // route. The modal dropped every one of them, and this fixture is why
        // nothing noticed: it did not contain them, so no assertion could.
        settings: {
          biorouter_provider: 'anthropic',
          biorouter_model: 'claude-opus-5',
          temperature: 0.3,
        },
        author: { contact: 'someone', metadata: undefined },
        version: '1.0.0',
      },
      error: undefined,
    };

    mockCreateWorkflow.mockResolvedValue({
      data: mockResponse,
      error: undefined,
      request: new globalThis.Request('http://localhost/test'),
      response: new globalThis.Response(),
    });
  });

  describe('Modal Rendering', () => {
    it('renders modal when open', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('create-workflow-modal')).toBeInTheDocument();
      await screen.findByTestId('form-state');
    });

    it('does not render when closed', () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} isOpen={false} />);

      expect(screen.queryByTestId('create-workflow-modal')).not.toBeInTheDocument();
    });

    it('renders modal header with close button', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('modal-header')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Close' })).toBeInTheDocument();
      await screen.findByTestId('form-state');
    });

    it('calls onClose when close button is clicked', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await user.click(screen.getByRole('button', { name: 'Close' }));
      expect(defaultProps.onClose).toHaveBeenCalled();
    });
  });

  describe('Analysis Workflow', () => {
    it('shows analyzing state initially', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('analyzing-state')).toBeInTheDocument();
      expect(screen.getByTestId('analyzing-title')).toBeInTheDocument();
      await screen.findByTestId('form-state');
    });

    it('displays analysis progress indicator', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('analysis-stage')).toBeInTheDocument();

      await waitFor(
        () => {
          const stageElement = screen.getByTestId('analysis-stage');
          expect(stageElement).toBeInTheDocument();
        },
        { timeout: 1000 }
      );
      await screen.findByTestId('form-state');
    });

    it('shows loading indicator during analysis', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('analysis-spinner')).toBeInTheDocument();
      await screen.findByTestId('form-state');
    });

    it('transitions to form state after analysis completes', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('form-state')).toBeInTheDocument();
        },
        { timeout: 3000 }
      );

      expect(screen.queryByTestId('analyzing-state')).not.toBeInTheDocument();
    });
  });

  describe('Form Pre-filling', () => {
    it('pre-fills form with analyzed data', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      // Wait for analysis to complete and form to be pre-filled
      await waitFor(
        () => {
          expect(screen.getByDisplayValue('Analyzed Workflow Title')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      expect(screen.getByDisplayValue('Analyzed description')).toBeInTheDocument();
      expect(screen.getByDisplayValue('Analyzed instructions with {{param1}}')).toBeInTheDocument();
      const promptInput = screen.getByTestId('prompt-input');
      expect(promptInput).toBeInTheDocument();
    });

    it('shows workflow form fields after analysis', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('workflow-form')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      expect(screen.getByTestId('title-input')).toBeInTheDocument();
      expect(screen.getByTestId('description-input')).toBeInTheDocument();
      expect(screen.getByTestId('instructions-input')).toBeInTheDocument();
      expect(screen.getByTestId('prompt-input')).toBeInTheDocument();
    });
  });

  describe('Form Interactions', () => {
    it('allows editing form fields', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('title-input')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      const titleInput = screen.getByTestId('title-input');
      await user.clear(titleInput);
      await user.type(titleInput, 'Modified Title');

      expect(screen.getByDisplayValue('Modified Title')).toBeInTheDocument();
    });

    it('validates required fields', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      const titleInput = screen.getByTestId('title-input');
      await user.clear(titleInput);

      const createButton = screen.getByTestId('create-workflow-button');
      expect(createButton).toBeDisabled();
    });
  });

  describe('Workflow Creation', () => {
    it('enables create button when form is valid', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          const createButton = screen.getByTestId('create-workflow-button');
          expect(createButton).toBeEnabled();
        },
        { timeout: 2000 }
      );
    });

    it('creates workflow and closes modal when form is submitted', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
        },
        { timeout: 2000 }
      );

      await user.click(screen.getByTestId('create-workflow-button'));

      await waitFor(() => {
        expect(defaultProps.onWorkflowCreated).toHaveBeenCalled();
        expect(defaultProps.onClose).toHaveBeenCalled();
      });
    });

    /**
     * The picker's skills come from the daemon's catalog (#113), so a skill
     * bundled inside an installed extension can be attached to a workflow —
     * which the renderer-side scan this replaced never saw.
     *
     * ⚠ Asserted because the modal *catches* a failed load and falls back to an
     * empty list. When the mock was incomplete this whole file still passed,
     * while every test in it silently ran with no skills to attach at all.
     */
    it('asks the daemon for the skill catalog when it opens', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);
      await waitFor(() => expect(mockSkillCatalog).toHaveBeenCalled());
    });

    it('saves generated extensions, knowledge bases, and skills from the analysis response', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
        },
        { timeout: 2000 }
      );

      await user.click(screen.getByTestId('create-workflow-button'));

      await waitFor(() => {
        expect(mockSaveWorkflow).toHaveBeenCalled();
      });

      expect(mockSaveWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          description: 'Analyzed description',
          extensions: [
            expect.objectContaining({
              name: 'skills',
              description: 'Load and use skills from relevant directories',
            }),
          ],
          knowledge_bases: {
            default: 'research-kb',
            visible: ['research-kb'],
          },
          skills: ['literature-review'],
        }),
        null
      );
    });

    /**
     * The generated `settings` block reaches the saved workflow.
     *
     * The modal read `formData.settings` when building the object but never
     * PREFILLED the form from the generated workflow, so the provider/model pin
     * the generator produces was silently dropped — the same conversation gave
     * a pinned workflow through the CLI and an unpinned one here.
     * CreateEditWorkflowModal has always prefilled settings; the two disagreed.
     */
    it('keeps the generated settings pin, author and version when saving', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
        },
        { timeout: 2000 }
      );

      await user.click(screen.getByTestId('create-workflow-button'));

      await waitFor(() => {
        expect(mockSaveWorkflow).toHaveBeenCalled();
      });

      expect(mockSaveWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          settings: {
            biorouter_provider: 'anthropic',
            biorouter_model: 'claude-opus-5',
            temperature: 0.3,
          },
          author: { contact: 'someone', metadata: undefined },
          version: '1.0.0',
        }),
        null
      );
    });
  });

  describe('Modal Footer', () => {
    it('shows cancel button in all states', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('cancel-button')).toBeInTheDocument();

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      expect(screen.getByTestId('cancel-button')).toBeInTheDocument();
    });

    it('calls onClose when cancel button is clicked', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await user.click(screen.getByTestId('cancel-button'));
      expect(defaultProps.onClose).toHaveBeenCalled();
    });

    it('shows different button states based on workflow stage', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      expect(screen.getByTestId('cancel-button')).toBeInTheDocument();
      expect(screen.queryByTestId('create-workflow-button')).not.toBeInTheDocument();

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      expect(screen.getByTestId('create-and-run-workflow-button')).toBeInTheDocument();
    });
  });

  describe('Error Handling', () => {
    it('handles analysis errors gracefully', async () => {
      render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId="" />);

      expect(screen.getByTestId('create-workflow-modal')).toBeInTheDocument();
      await screen.findByTestId('form-state');
    });

    it('handles form validation errors', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('title-input')).toBeInTheDocument();
        },
        { timeout: 2000 }
      );

      await user.clear(screen.getByTestId('title-input'));
      await user.clear(screen.getByTestId('description-input'));
      await user.clear(screen.getByTestId('instructions-input'));

      const createButton = screen.getByTestId('create-workflow-button');
      expect(createButton).toBeDisabled();
    });
  });

  /**
   * Issue #56 Task 58: the modal reads the chat's knowledge-base selection with
   * `GET /knowledge/active`, and naming a PRIVATE chat there is on the daemon's
   * reach gate. The read carried no proof, was refused, and the modal fell back
   * to "nothing is hidden, nothing is primary" — so a workflow captured from a
   * private chat took EVERY base, with whichever came first as its default.
   */
  describe('from a private chat', () => {
    const PRIVATE_CHAT = 'private-session-id';
    const kb = (id: string) => ({ id, name: id, color: '#cf6d47', created_at: '' });

    beforeEach(() => {
      Object.assign(window, {
        electron: {
          listSkillDirs: vi.fn().mockResolvedValue([]),
          getUserActionKey: vi.fn(async () => USER_ACTION_KEY),
        },
      });
      mockListBases.mockResolvedValue({
        data: [kb('soul'), kb('lab-notes'), kb('grant-drafts')],
        error: undefined,
      } as never);
      mockGetActive.mockImplementation(
        reachGatedGetActive([PRIVATE_CHAT], () => ({
          kb_ids: ['lab-notes', 'soul'],
          primary_kb: 'lab-notes',
          active_kb: 'lab-notes',
          hidden_kbs: ['grant-drafts'],
        })) as never
      );
      // A generation with no knowledge-base block of its own, so the chat's
      // selection is the only place the saved workflow can take its bases from.
      mockCreateWorkflow.mockResolvedValue({
        data: {
          workflow: {
            title: 'Analyzed Workflow Title',
            description: 'Analyzed description',
            instructions: 'Analyzed instructions',
          },
          error: undefined,
        },
        error: undefined,
        request: new globalThis.Request('http://localhost/test'),
        response: new globalThis.Response(),
      });
    });

    afterEach(() => {
      // `clearAllMocks` keeps implementations, so put back what the rest of the
      // file expects rather than leak this chat's gate into it.
      mockListBases.mockResolvedValue({ data: [], error: undefined } as never);
      mockGetActive.mockResolvedValue({
        data: { active_kb: null, hidden_kbs: [] },
        error: undefined,
      } as never);
    });

    it("captures the chat's knowledge bases and its primary, not every base", async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId={PRIVATE_CHAT} />);

      await waitFor(
        () => {
          expect(screen.getByTestId('create-workflow-button')).toBeEnabled();
        },
        { timeout: 2000 }
      );
      await user.click(screen.getByTestId('create-workflow-button'));
      await waitFor(() => {
        expect(mockSaveWorkflow).toHaveBeenCalled();
      });

      expect(mockSaveWorkflow).toHaveBeenCalledWith(
        expect.objectContaining({
          knowledge_bases: { default: 'lab-notes', visible: ['soul', 'lab-notes'] },
        }),
        null
      );
      expect(mockGetActive).toHaveBeenCalledWith(
        expect.objectContaining({
          query: { session_id: PRIVATE_CHAT },
          headers: { 'X-User-Action': USER_ACTION_KEY },
        })
      );
    });

    /**
     * With the proof attached, a read that still fails is a genuine error — a
     * surface that cannot prove the person, a dropped connection, an older
     * daemon — and none of those said "nothing is hidden, nothing is primary".
     * The modal saved it as exactly that: every base, with whichever came first
     * as the default, written into a workflow that outlives the chat.
     *
     * A failed read now captures nothing. The daemon's own block, which the
     * generation carries whenever a base exists, is what the workflow keeps;
     * without one the workflow has nothing to say about knowledge bases, and
     * the picker says why nothing is selected.
     */
    describe('when its selection cannot be read', () => {
      const SELECTION_UNREAD =
        "Could not load this chat's knowledge bases, so none were selected automatically.";
      const withGeneratedKnowledgeBases = {
        data: {
          workflow: {
            title: 'Analyzed Workflow Title',
            description: 'Analyzed description',
            instructions: 'Analyzed instructions',
            // The daemon reads the chat's selection itself, past no gate.
            knowledge_bases: { default: 'lab-notes', visible: ['lab-notes', 'soul'] },
          },
          error: undefined,
        },
        error: undefined,
        request: new globalThis.Request('http://localhost/test'),
        response: new globalThis.Response(),
      };
      let warn: MockInstance;

      beforeEach(() => {
        // A preload with no bridge: `userActionHeaders()` sends no proof, and
        // the gate answers the way it answers any caller that has none.
        Object.assign(window, {
          electron: { listSkillDirs: vi.fn().mockResolvedValue([]) },
        });
        warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      });

      afterEach(() => {
        warn.mockRestore();
      });

      it('captures no knowledge bases rather than every base', async () => {
        const user = userEvent.setup();
        render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId={PRIVATE_CHAT} />);

        const saved = await saveTheWorkflow(user);

        expect(saved?.knowledge_bases).toBeUndefined();
        expect(warn).toHaveBeenCalledWith('Knowledge selection not read:', expect.any(String));
      });

      it('says why no knowledge base is selected', async () => {
        const user = userEvent.setup();
        render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId={PRIVATE_CHAT} />);

        // Nothing was captured, so nothing opens the advanced options for us.
        await user.click(await screen.findByText('Advanced options'));

        const notice = screen.getByText(SELECTION_UNREAD);
        expect(notice.closest('[role="status"]')).not.toBeNull();
        expect(screen.getByText('No KBs selected')).toBeInTheDocument();
      });

      // The race the old code lost: a read answering after the generation wrote
      // every base over the daemon's own block. Only inside the analysis window,
      // though — once the form is up, the effect has been torn down and a late
      // answer is dropped — so the read is released while the spinner still runs.
      it("keeps the generation's knowledge bases when the failed read answers after it", async () => {
        const user = userEvent.setup();
        const bases = deferred<unknown>();
        mockListBases.mockReturnValue(bases.promise as never);
        mockCreateWorkflow.mockResolvedValue(withGeneratedKnowledgeBases);
        render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId={PRIVATE_CHAT} />);

        await settleReads();
        expect(screen.getByTestId('analyzing-state')).toBeInTheDocument();
        bases.resolve({ data: [kb('soul'), kb('lab-notes'), kb('grant-drafts')] });
        await settleReads();

        expect(await screen.findByTestId('form-state')).toBeInTheDocument();
        expect(screen.getByText('2 KBs selected')).toBeInTheDocument();
        expect(screen.queryByText(SELECTION_UNREAD)).not.toBeInTheDocument();
        const saved = await saveTheWorkflow(user);
        expect(saved?.knowledge_bases).toEqual({
          default: 'lab-notes',
          visible: ['lab-notes', 'soul'],
        });
        expect(warn).toHaveBeenCalledWith('Knowledge selection not read:', expect.any(String));
      });

      // The usual order in the app: the read fails at once, and the generation,
      // which waits on a model, brings the daemon's block afterwards. That block
      // IS the chat's selection, so the picker must not go on calling it unread.
      it('stops calling the selection unread once the generation brings it', async () => {
        const user = userEvent.setup();
        const generation = deferred<typeof withGeneratedKnowledgeBases>();
        mockCreateWorkflow.mockReturnValue(generation.promise as never);
        render(<CreateWorkflowFromSessionModal {...defaultProps} sessionId={PRIVATE_CHAT} />);

        await waitFor(() => expect(mockGetActive).toHaveBeenCalled());
        await settleReads();
        await act(async () => {
          generation.resolve(withGeneratedKnowledgeBases);
        });

        // The generation's block opens the advanced options on its own.
        expect(await screen.findByText('2 KBs selected')).toBeInTheDocument();
        expect(screen.queryByText(SELECTION_UNREAD)).not.toBeInTheDocument();
        const saved = await saveTheWorkflow(user);
        expect(saved?.knowledge_bases).toEqual({
          default: 'lab-notes',
          visible: ['lab-notes', 'soul'],
        });
      });
    });
  });

  /**
   * A chat with no primary knowledge base gives a workflow with no primary.
   *
   * The modal filled the gap with the first visible base, on the read path and
   * the generation path alike, and `apply_knowledge_selection`
   * (`crates/biorouter/src/workflow/runtime.rs`) turns a saved `default` into
   * `PrimaryUpdate::Set`. So every chat the workflow started got a write target
   * the chat it was captured from never had. The daemon's rule is the opposite
   * (`plan_knowledge_selection`): the primary comes only from `default`, and is
   * never inferred from `visible`.
   */
  describe("the workflow's primary knowledge base", () => {
    const kb = (id: string) => ({ id, name: id, color: '#cf6d47', created_at: '' });
    /** What the daemon answers for this chat: two bases, and neither is primary. */
    const NO_PRIMARY = {
      kb_ids: ['lab-notes', 'soul'],
      primary_kb: null,
      active_kb: null,
      hidden_kbs: ['grant-drafts'],
    };
    const generation = (knowledgeBases?: WorkflowKnowledgeBases) => ({
      data: {
        workflow: {
          title: 'Analyzed Workflow Title',
          description: 'Analyzed description',
          instructions: 'Analyzed instructions',
          ...(knowledgeBases ? { knowledge_bases: knowledgeBases } : {}),
        },
        error: undefined,
      },
      error: undefined,
      request: new globalThis.Request('http://localhost/test'),
      response: new globalThis.Response(),
    });
    /** Leave the generation's block as the only statement of the chat's selection. */
    const failTheSelectionRead = () =>
      mockGetActive.mockResolvedValue({ data: undefined, error: 'Failed to fetch' } as never);
    let warn: MockInstance;

    beforeEach(() => {
      warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
      mockListBases.mockResolvedValue({
        data: [kb('soul'), kb('lab-notes'), kb('grant-drafts')],
        error: undefined,
      } as never);
      mockGetActive.mockResolvedValue({ data: NO_PRIMARY, error: undefined } as never);
      // No block of its own, so the chat's selection as the modal read it is the
      // only place the saved workflow can take its bases from.
      mockCreateWorkflow.mockResolvedValue(generation());
    });

    afterEach(() => {
      warn.mockRestore();
      mockListBases.mockResolvedValue({ data: [], error: undefined } as never);
      mockGetActive.mockResolvedValue({
        data: { active_kb: null, hidden_kbs: [] },
        error: undefined,
      } as never);
    });

    it('saves no primary when the chat has none', async () => {
      const user = userEvent.setup();
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      const saved = await saveTheWorkflow(user);

      expect(mockGetActive).toHaveBeenCalledWith(
        expect.objectContaining({ query: { session_id: defaultProps.sessionId } })
      );
      expect(saved?.knowledge_bases).toEqual({ default: null, visible: ['soul', 'lab-notes'] });
    });

    // The daemon never sends `default: null` itself: the field is
    // `skip_serializing_if = "Option::is_none"`, so its own block for a chat
    // with no primary has no `default` at all. The two mean the same thing.
    it.each<[string, WorkflowKnowledgeBases]>([
      ['null', { default: null, visible: ['lab-notes', 'soul'] }],
      ['absent', { visible: ['lab-notes', 'soul'] }],
    ])('saves no primary when the generated block has none (default %s)', async (_, block) => {
      const user = userEvent.setup();
      failTheSelectionRead();
      mockCreateWorkflow.mockResolvedValue(generation(block));
      render(<CreateWorkflowFromSessionModal {...defaultProps} />);

      const saved = await saveTheWorkflow(user);

      expect(saved?.knowledge_bases).toEqual({ default: null, visible: ['lab-notes', 'soul'] });
    });

    /**
     * A primary that is not among the bases the workflow will see is dropped,
     * not unioned into them the way `plan_knowledge_selection` unions a
     * `default` missing from `visible`. The daemon does that for a workflow
     * somebody wrote, whose author plainly meant that `default`. Nobody wrote
     * this one: a captured primary outside its own set is an inconsistent read.
     */
    describe('a primary outside the visible bases', () => {
      // The modal reads the base list and the selection as two requests, which
      // can answer in either order, so a base created or deleted between them
      // leaves the selection naming a primary the list does not hold. Unioning it
      // in could save a base that no longer exists as the default, and
      // `set_visible_kbs` refuses a primary outside the set, so every chat the
      // workflow starts would then fail.
      it("is not saved from the chat's selection", async () => {
        const user = userEvent.setup();
        mockGetActive.mockResolvedValue({
          data: {
            kb_ids: ['lab-notes', 'new-notes', 'soul'],
            primary_kb: 'new-notes',
            active_kb: 'new-notes',
            hidden_kbs: ['grant-drafts'],
          },
          error: undefined,
        } as never);
        render(<CreateWorkflowFromSessionModal {...defaultProps} />);

        const saved = await saveTheWorkflow(user);

        expect(saved?.knowledge_bases).toEqual({ default: null, visible: ['soul', 'lab-notes'] });
      });

      // The daemon's block is one locked snapshot whose primary is always a
      // member of its set (`selection_unlocked`), so this guards against a block
      // that breaks that; today's daemon never sends one.
      it('is not saved from the generated block', async () => {
        const user = userEvent.setup();
        failTheSelectionRead();
        mockCreateWorkflow.mockResolvedValue(
          generation({ default: 'grant-drafts', visible: ['lab-notes', 'soul'] })
        );
        render(<CreateWorkflowFromSessionModal {...defaultProps} />);

        const saved = await saveTheWorkflow(user);

        expect(saved?.knowledge_bases).toEqual({ default: null, visible: ['lab-notes', 'soul'] });
      });
    });
  });
});
