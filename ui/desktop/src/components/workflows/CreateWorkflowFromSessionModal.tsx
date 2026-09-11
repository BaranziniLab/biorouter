import { useState, useEffect, useRef } from 'react';
import { useForm } from '@tanstack/react-form';
import { Workflow, WorkflowKnowledgeBases } from '../../workflow';
import { Save, Play, Loader2 } from '../icons/app-icons';
import { Button } from '../ui/button';
import { WorkflowFormFields } from './shared/WorkflowFormFields';
import { WorkflowFormData } from './shared/workflowFormSchema';
import { createWorkflow, getSessionExtensions, listBases } from '../../api/sdk.gen';
import { readKnowledgeSelection } from '../knowledge/knowledgeSelection';
import { WorkflowParameter } from './shared/workflowFormSchema';
import { toastError } from '../../toasts';
import { saveWorkflow } from '../../workflow/workflow_management';
import type { ExtensionConfig, Manifest } from '../../api';
import { fetchSkillCatalog, pickerBundles, standaloneSkills } from '../skills/useSkillCatalog';
import type { WorkflowResourceItem } from './shared/WorkflowResourcePicker';
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '../ui/dialog';
import { storeSubscriptionCleanup } from '../../utils/storeSubscription';

interface CreateWorkflowFromSessionModalProps {
  isOpen: boolean;
  onClose: () => void;
  sessionId: string;
  onWorkflowCreated?: (workflow: Workflow) => void;
}

const KNOWLEDGE_SELECTION_UNREAD =
  "Could not load this chat's knowledge bases, so none were selected automatically.";

export default function CreateWorkflowFromSessionModal({
  isOpen,
  onClose,
  sessionId,
  onWorkflowCreated,
}: CreateWorkflowFromSessionModalProps) {
  const [isCreating, setIsCreating] = useState(false);
  const [isAnalyzing, setIsAnalyzing] = useState(false);
  const [analysisStage, setAnalysisStage] = useState<string>('');
  const [hasAnalyzed, setHasAnalyzed] = useState(false);
  const [workflowExtensions, setWorkflowExtensions] = useState<ExtensionConfig[]>([]);
  const [knowledgeBaseItems, setKnowledgeBaseItems] = useState<WorkflowResourceItem[]>([]);
  const [workflowKnowledgeBaseIds, setWorkflowKnowledgeBaseIds] = useState<string[]>([]);
  const [defaultKnowledgeBaseId, setDefaultKnowledgeBaseId] = useState<string | null>(null);
  // The chat's selection could not be read, and the generation has not brought
  // the daemon's own copy of it: nothing was captured, and the picker says so.
  const [knowledgeSelectionUnread, setKnowledgeSelectionUnread] = useState(false);
  const [skillItems, setSkillItems] = useState<WorkflowResourceItem[]>([]);
  const [workflowSkillIds, setWorkflowSkillIds] = useState<string[]>([]);
  const generatedResourcesRef = useRef<{
    extensions?: ExtensionConfig[];
    knowledgeBases?: WorkflowKnowledgeBases;
    skills?: string[];
    author?: Workflow['author'];
    version?: string;
  }>({});
  const resourceEditsRef = useRef({
    extensions: false,
    knowledgeBases: false,
    skills: false,
  });

  // Initialize form with empty values for new workflow
  const form = useForm({
    defaultValues: {
      title: '',
      description: '',
      instructions: '',
      prompt: '',
      activities: [] as string[],
      parameters: [] as WorkflowParameter[],
      jsonSchema: '',
      workflowName: '',
      global: true,
    } as WorkflowFormData,
    onSubmit: async ({ value }) => {
      await handleCreateWorkflow(value);
    },
  });

  // Track form validity with state to make it reactive
  const [isFormValid, setIsFormValid] = useState(false);

  // Analyze messages and prefill form when modal opens
  useEffect(() => {
    if (isOpen && sessionId && !hasAnalyzed) {
      let cancelled = false;
      let completionTimeout: ReturnType<typeof setTimeout> | undefined;
      setIsAnalyzing(true);

      // Create a sequence of analysis stages for better UX
      const stages = [
        'Reading your chat...',
        'Identifying key patterns...',
        'Extracting main topics...',
        'Generating workflow structure...',
        'Finalizing details...',
      ];

      let currentStageIndex = 0;
      setAnalysisStage(stages[0]);

      // Update stage every 800ms
      const stageInterval = setInterval(() => {
        currentStageIndex = (currentStageIndex + 1) % stages.length;
        setAnalysisStage(stages[currentStageIndex]);
      }, 800);

      // Pre-select session extensions immediately — independent of workflow analysis
      getSessionExtensions({ path: { session_id: sessionId }, throwOnError: false }).then((res) => {
        if (cancelled) return;
        if (res.data?.extensions) {
          setWorkflowExtensions(res.data.extensions);
        }
      });

      Promise.all([
        listBases({ throwOnError: false }),
        // Issue #56 Task 58: this modal opens from the chat it names, and the
        // read carries the user's proof, which a GET naming a PRIVATE chat
        // needs. `null` when the read failed anyway.
        readKnowledgeSelection(sessionId),
      ]).then(([basesRes, selection]) => {
        if (cancelled) return;
        const bases: Manifest[] = basesRes.data ?? [];
        setKnowledgeBaseItems(
          bases.map((base) => ({
            id: base.id,
            label: base.name,
            description: base.id,
          }))
        );

        if (!selection) {
          // A failed read is not "nothing is hidden, nothing is primary", and
          // capturing it as that saved every base, with whichever came first as
          // the default, into a workflow that outlives this chat. Capture
          // nothing instead. The generation carries the daemon's own block
          // whenever a base exists, and the save path falls back to it; with
          // none, the picker says why nothing is selected. A block that already
          // arrived needs no such note, and must not be overwritten.
          if (!generatedResourcesRef.current.knowledgeBases) setKnowledgeSelectionUnread(true);
          return;
        }
        const visible = bases
          .filter((base) => !selection.hiddenKbIds.has(base.id))
          .map((base) => base.id);
        const primary = selection.primaryKbId;
        const defaultId = primary && visible.includes(primary) ? primary : (visible[0] ?? null);
        setWorkflowKnowledgeBaseIds(visible);
        setDefaultKnowledgeBaseId(defaultId);
      });

      // The daemon's catalog, so a skill bundled inside an installed extension
      // can be attached to a workflow like any other (#113).
      fetchSkillCatalog()
        .then((view) => {
          if (cancelled) return;
          const bundleItems: WorkflowResourceItem[] = pickerBundles(view).map((bundle) => ({
            id: bundle.name,
            label: bundle.displayName,
            description: bundle.skills.join(', '),
            badge: 'bundle',
          }));
          const singleItems: WorkflowResourceItem[] = standaloneSkills(view).map((skill) => ({
            id: skill.name,
            label: skill.name,
            description: skill.description,
          }));
          setSkillItems([...bundleItems, ...singleItems]);
        })
        .catch((error) => {
          if (cancelled) return;
          console.warn('Failed to load workflow skills:', error);
          setSkillItems([]);
        });

      // Analyze the conversation to generate a suggested workflow
      createWorkflow({
        body: { session_id: sessionId },
        throwOnError: true,
      })
        .then((response) => {
          if (cancelled) return;
          clearInterval(stageInterval);
          setAnalysisStage('Complete!');

          if (response.data?.workflow) {
            const workflow = response.data.workflow;

            // Prefill the form with the analyzed workflow information
            form.setFieldValue('title', workflow.title || '');
            form.setFieldValue('description', workflow.description || '');
            form.setFieldValue('instructions', workflow.instructions || '');
            form.setFieldValue('activities', workflow.activities || []);
            form.setFieldValue('parameters', workflow.parameters || []);
            form.setFieldValue('prompt', workflow.prompt || '');

            // The generator DOES produce a settings block — it pins the
            // provider, model and temperature the workflow was captured under.
            // Not prefilling it here silently dropped that pin, so the same
            // conversation yielded a pinned workflow through the CLI and an
            // unpinned one through this modal. CreateEditWorkflowModal has
            // always prefilled settings; the two disagreed.
            if (workflow.settings) {
              form.setFieldValue('settings', {
                biorouter_provider: workflow.settings.biorouter_provider ?? undefined,
                biorouter_model: workflow.settings.biorouter_model ?? undefined,
                temperature: workflow.settings.temperature ?? undefined,
              });
            }

            // `author` and `version` have no form fields — they are facts about
            // the capture, not things the user edits — so they are held here and
            // written back on save. Without this the modal dropped both, and the
            // author is the one field the HTTP route goes out of its way to
            // attach.
            generatedResourcesRef.current.author = workflow.author ?? undefined;
            generatedResourcesRef.current.version = workflow.version ?? undefined;

            if (workflow.response?.json_schema) {
              form.setFieldValue(
                'jsonSchema',
                JSON.stringify(workflow.response.json_schema, null, 2)
              );
            }

            if (workflow.extensions && workflow.extensions.length > 0) {
              generatedResourcesRef.current.extensions = workflow.extensions;
              setWorkflowExtensions(workflow.extensions);
            }

            if (workflow.knowledge_bases) {
              // The daemon read the chat's selection itself, so whatever this
              // modal's own read said, the selection is known now.
              setKnowledgeSelectionUnread(false);
              const visible = workflow.knowledge_bases.visible ?? [];
              generatedResourcesRef.current.knowledgeBases = {
                default:
                  workflow.knowledge_bases.default &&
                  visible.includes(workflow.knowledge_bases.default)
                    ? workflow.knowledge_bases.default
                    : (visible[0] ?? null),
                visible,
              };
              setWorkflowKnowledgeBaseIds(visible);
              setDefaultKnowledgeBaseId(
                workflow.knowledge_bases.default &&
                  visible.includes(workflow.knowledge_bases.default)
                  ? workflow.knowledge_bases.default
                  : (visible[0] ?? null)
              );
            }

            if (workflow.skills && workflow.skills.length > 0) {
              generatedResourcesRef.current.skills = workflow.skills;
              setWorkflowSkillIds(workflow.skills);
            }
          } else {
            console.error('No workflow in response:', response);
          }
        })
        .catch((error) => {
          if (cancelled) return;
          console.error('Failed to analyze messages:', error);
          setAnalysisStage('Analysis failed');
        })
        .finally(() => {
          if (cancelled) return;
          clearInterval(stageInterval);
          completionTimeout = setTimeout(() => {
            if (cancelled) return;
            setHasAnalyzed(true);
            setIsAnalyzing(false);
            setAnalysisStage('');
          }, 500);
        });

      return () => {
        cancelled = true;
        clearInterval(stageInterval);
        if (completionTimeout) clearTimeout(completionTimeout);
      };
    }
    return undefined;
  }, [isOpen, sessionId, hasAnalyzed, form]);

  // Reset state when modal closes
  useEffect(() => {
    if (!isOpen) {
      setHasAnalyzed(false);
      setIsAnalyzing(false);
      setAnalysisStage('');
      setWorkflowExtensions([]);
      setKnowledgeBaseItems([]);
      setWorkflowKnowledgeBaseIds([]);
      setDefaultKnowledgeBaseId(null);
      setKnowledgeSelectionUnread(false);
      setSkillItems([]);
      setWorkflowSkillIds([]);
      generatedResourcesRef.current = {};
      resourceEditsRef.current = {
        extensions: false,
        knowledgeBases: false,
        skills: false,
      };
    }
  }, [isOpen]);

  // Subscribe to form changes using the form's subscribe method
  useEffect(() => {
    const subscription = form.store.subscribe(() => {
      const hasTitle = form.state.values.title?.trim();
      const hasDescription = form.state.values.description?.trim();
      const hasInstructions = form.state.values.instructions?.trim();
      const valid = !!(hasTitle && hasDescription && hasInstructions);

      setIsFormValid(valid);
    });

    // Initial validation check
    const hasTitle = form.state.values.title?.trim();
    const hasDescription = form.state.values.description?.trim();
    const hasInstructions = form.state.values.instructions?.trim();
    const valid = !!(hasTitle && hasDescription && hasInstructions);
    setIsFormValid(valid);

    return storeSubscriptionCleanup(subscription);
  }, [form]);

  const handleCreateWorkflow = async (formData: WorkflowFormData, runAfterSave = false) => {
    if (isCreating || !isFormValid) {
      return;
    }

    setIsCreating(true);
    try {
      // Build settings if any model override was specified
      const settingsConfig =
        formData.settings?.biorouter_provider ||
        formData.settings?.biorouter_model ||
        formData.settings?.temperature !== undefined
          ? {
              biorouter_provider: formData.settings?.biorouter_provider || undefined,
              biorouter_model: formData.settings?.biorouter_model || undefined,
              temperature: formData.settings?.temperature,
            }
          : undefined;

      // Create the workflow object from form data
      const generatedResources = generatedResourcesRef.current;
      const selectedExtensions =
        workflowExtensions.length > 0 || resourceEditsRef.current.extensions
          ? workflowExtensions
          : (generatedResources.extensions ?? []);
      const selectedKnowledgeBaseIds =
        workflowKnowledgeBaseIds.length > 0 || resourceEditsRef.current.knowledgeBases
          ? workflowKnowledgeBaseIds
          : (generatedResources.knowledgeBases?.visible ?? []);
      const selectedDefaultKnowledgeBaseId =
        defaultKnowledgeBaseId ??
        (resourceEditsRef.current.knowledgeBases
          ? null
          : (generatedResources.knowledgeBases?.default ?? null));
      const selectedSkillIds =
        workflowSkillIds.length > 0 || resourceEditsRef.current.skills
          ? workflowSkillIds
          : (generatedResources.skills ?? []);
      const knowledgeBases: WorkflowKnowledgeBases | undefined =
        selectedKnowledgeBaseIds.length > 0 || selectedDefaultKnowledgeBaseId
          ? {
              default: selectedDefaultKnowledgeBaseId,
              visible: selectedKnowledgeBaseIds,
            }
          : undefined;

      const workflow: Workflow = {
        title: formData.title,
        description: formData.description,
        instructions: formData.instructions,
        prompt: formData.prompt || undefined,
        activities: formData.activities.filter((activity) => activity.trim() !== ''),
        parameters: formData.parameters.map((param) => ({
          key: param.key,
          input_type: param.input_type || 'string',
          requirement: param.requirement,
          description: param.description,
          ...(param.requirement === 'optional' && param.default ? { default: param.default } : {}),
          ...(param.input_type === 'select' && param.options
            ? {
                options: param.options.filter((opt: string) => opt.trim() !== ''),
              }
            : {}),
        })),
        response:
          formData.jsonSchema && formData.jsonSchema.trim()
            ? {
                json_schema: JSON.parse(formData.jsonSchema),
              }
            : undefined,
        settings: settingsConfig,
        extensions:
          selectedExtensions.length > 0
            ? (selectedExtensions.map((ext) =>
                'envs' in ext ? { ...ext, envs: undefined } : ext
              ) as ExtensionConfig[])
            : undefined,
        knowledge_bases: knowledgeBases,
        skills: selectedSkillIds.length > 0 ? selectedSkillIds : undefined,
        // Captured at generation time; see generatedResourcesRef above.
        author: generatedResources.author,
        ...(generatedResources.version ? { version: generatedResources.version } : {}),
      };

      let workflowId = await saveWorkflow(workflow, null);

      onWorkflowCreated?.(workflow);
      onClose();

      if (runAfterSave) {
        window.electron.createChatWindow(
          undefined,
          undefined,
          undefined,
          undefined,
          undefined,
          workflowId
        );
      }
    } catch (error) {
      console.error('Failed to create workflow:', error);
      toastError({
        title: 'Failed to create workflow',
        msg: error instanceof Error ? error.message : 'Could not create the workflow. Try again.',
      });
    } finally {
      setIsCreating(false);
    }
  };

  if (!isOpen) return null;

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && !isCreating && onClose()}>
      <DialogContent
        dismissible={!isCreating}
        className="flex h-full max-h-[90vh] w-full max-w-4xl flex-col gap-0 overflow-hidden p-0 sm:max-w-[calc(100%-2rem)] lg:max-w-4xl"
        data-testid="create-workflow-modal"
      >
        {/* Header */}
        <div
          className="flex shrink-0 items-center justify-between px-6 pb-3 pt-5 pr-14"
          data-testid="modal-header"
        >
          <div>
            <DialogTitle>Create workflow from this chat</DialogTitle>
            <DialogDescription className="text-supporting text-text-muted mt-0.5">
              Create a reusable workflow based on your current chat
            </DialogDescription>
          </div>
        </div>

        {/* Content */}
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4" data-testid="modal-content">
          {isAnalyzing ? (
            <div
              className="flex flex-col items-center justify-center h-full min-h-[300px] gap-3"
              data-testid="analyzing-state"
            >
              <Loader2
                className="w-6 h-6 animate-spin text-text-muted"
                data-testid="analysis-spinner"
              />
              <div className="text-center">
                <p className="text-label text-text-default" data-testid="analyzing-title">
                  Analyzing your chat
                </p>
                <p className="text-supporting text-text-muted mt-1" data-testid="analysis-stage">
                  {analysisStage}
                </p>
              </div>
            </div>
          ) : (
            <div data-testid="form-state">
              <WorkflowFormFields
                form={form}
                extensions={workflowExtensions}
                onExtensionsChange={(extensions) => {
                  resourceEditsRef.current.extensions = true;
                  setWorkflowExtensions(extensions);
                }}
                knowledgeBaseItems={knowledgeBaseItems}
                selectedKnowledgeBaseIds={workflowKnowledgeBaseIds}
                onKnowledgeBaseIdsChange={(ids) => {
                  resourceEditsRef.current.knowledgeBases = true;
                  setWorkflowKnowledgeBaseIds(ids);
                  if (defaultKnowledgeBaseId && !ids.includes(defaultKnowledgeBaseId)) {
                    setDefaultKnowledgeBaseId(ids[0] ?? null);
                  } else if (!defaultKnowledgeBaseId && ids.length > 0) {
                    setDefaultKnowledgeBaseId(ids[0]);
                  }
                }}
                defaultKnowledgeBaseId={defaultKnowledgeBaseId}
                onDefaultKnowledgeBaseIdChange={(id) => {
                  resourceEditsRef.current.knowledgeBases = true;
                  setDefaultKnowledgeBaseId(id);
                }}
                knowledgeBaseNotice={
                  knowledgeSelectionUnread ? KNOWLEDGE_SELECTION_UNREAD : undefined
                }
                skillItems={skillItems}
                selectedSkillIds={workflowSkillIds}
                onSkillIdsChange={(ids) => {
                  resourceEditsRef.current.skills = true;
                  setWorkflowSkillIds(ids);
                }}
              />
            </div>
          )}
        </div>

        {/* Footer */}
        <div
          className="flex shrink-0 items-center justify-between px-6 pb-5 pt-3"
          data-testid="modal-footer"
        >
          <Button
            onClick={onClose}
            variant="ghost"
            disabled={isCreating}
            className="rounded-element px-4 py-2 text-text-muted transition-colors hover:bg-overlay-hover"
            data-testid="cancel-button"
          >
            Cancel
          </Button>

          {!isAnalyzing && (
            <div className="flex gap-3">
              {/* V7 — this was `ghost` repainted into `secondary` by hand: a
                  `bg-background-medium` ground, a `rounded-element`, and a
                  `hover:bg-overlay-hover` that `tint-interactive` already owns.
                  The variant meaning "the quieter of two committing actions"
                  has a name, so it uses the name. */}
              <Button
                onClick={() => form.handleSubmit()}
                disabled={!isFormValid || isCreating}
                variant="secondary"
                data-testid="create-workflow-button"
              >
                <Save className="w-4 h-4" />
                {isCreating ? 'Creating…' : 'Create workflow'}
              </Button>
              <Button
                onClick={() => handleCreateWorkflow(form.state.values, true)}
                disabled={!isFormValid || isCreating}
                variant="default"
                data-testid="create-and-run-workflow-button"
              >
                <Play className="w-4 h-4" />
                {isCreating ? 'Creating…' : 'Create & Run'}
              </Button>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
