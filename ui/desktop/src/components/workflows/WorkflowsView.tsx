import { useState, useEffect, useMemo } from 'react';
import { listSavedWorkflows } from '../../workflow/workflow_management';
import { AlertCircle, Play, Plus, Upload } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { ScrollArea } from '../ui/scroll-area';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Skeleton } from '../ui/skeleton';
import { FilterInput } from '../ui/filter-input';
import { InfoTip } from '../ui/info-tip';
import { IconAction, RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { ModalShell } from '../ModalShell';
import { toastSuccess, toastError } from '../../toasts';
import { errorMessage } from '../../utils/conversionUtils';
import {
  deleteWorkflow,
  WorkflowManifest,
  startAgent,
  setWorkflowSlashCommand,
  workflowToYaml,
} from '../../api';
// Through `schedule.ts`, never the generated `scheduleWorkflow`: that module is
// where every schedule write carries the person's proof and reads a refusal.
import { scheduleWorkflowById } from '../../schedule';
import ImportWorkflowForm from './ImportWorkflowForm';
import CreateEditWorkflowModal from './CreateEditWorkflowModal';
import { generateDeepLink, Workflow } from '../../workflow';
import { useNavigation } from '../../hooks/useNavigation';
import { CronPicker } from '../schedule/CronPicker';
import { focusDialogOnOpen } from '../schedule/dialogFocus';
import { getInitialWorkingDir } from '../../utils/workingDir';
import { startChatFailureNotice } from '../../utils/startChatFailure';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import BuiltInBadge from '../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinWorkflow } from '../../utils/builtins';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { useConfig } from '../ConfigContext';
import { userActionHeaders } from '../../utils/userAction';
import { rowTime, rowTimeTitle } from './shared/rowTime';
import { cronSentence, shortCron } from './shortCron';
import { workflowsCopy as copy } from './copy';

const WorkflowIcon = ENTITY_ICONS.workflow;

/** `last_modified` as a Date (the daemon sends an RFC 3339 string), or null. */
function modifiedAt(lastModified: string | null | undefined): Date | null {
  if (!lastModified) return null;
  const date = new Date(lastModified);
  return Number.isNaN(date.getTime()) ? null : date;
}

/**
 * The selector a row click must ignore: a click that landed on one of the row's own controls
 * (Run, `⋯`, a badge's help) is that control's, not "open this workflow".
 */
const ROW_CONTROL_SELECTOR =
  'button, a[href], input, select, textarea, [role="menuitem"], [tabindex]:not([tabindex="-1"])';

export default function WorkflowsView() {
  const setView = useNavigation();
  const { refreshConfig } = useConfig();
  const [savedWorkflows, setSavedWorkflows] = useState<WorkflowManifest[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedWorkflow, setSelectedWorkflow] = useState<WorkflowManifest | null>(null);
  const [showEditor, setShowEditor] = useState(false);
  const [workflowToDelete, setWorkflowToDelete] = useState<WorkflowManifest | null>(null);
  const [isDeletingWorkflow, setIsDeletingWorkflow] = useState(false);

  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [showImportDialog, setShowImportDialog] = useState(false);

  const [showScheduleDialog, setShowScheduleDialog] = useState(false);
  const [scheduleWorkflowManifest, setScheduleWorkflowManifest] = useState<WorkflowManifest | null>(
    null
  );
  const [scheduleCron, setScheduleCron] = useState<string>('');
  const [isSavingSchedule, setIsSavingSchedule] = useState(false);

  const [showSlashCommandDialog, setShowSlashCommandDialog] = useState(false);
  const [slashCommandWorkflowManifest, setSlashCommandWorkflowManifest] =
    useState<WorkflowManifest | null>(null);
  const [slashCommand, setSlashCommand] = useState<string>('');
  const [slashCommandError, setSlashCommandError] = useState('');
  const [isSavingSlashCommand, setIsSavingSlashCommand] = useState(false);
  const [scheduleValid, setScheduleIsValid] = useState(true);

  const [searchTerm, setSearchTerm] = useState('');

  const filteredWorkflows = useMemo(() => {
    if (!searchTerm) return savedWorkflows;

    const searchLower = searchTerm.toLowerCase();
    return savedWorkflows.filter((workflowManifest) => {
      const { workflow, slash_command } = workflowManifest;
      const title = workflow.title?.toLowerCase() || '';
      const description = workflow.description?.toLowerCase() || '';
      const slashCmd = slash_command?.toLowerCase() || '';

      return (
        title.includes(searchLower) ||
        description.includes(searchLower) ||
        slashCmd.includes(searchLower)
      );
    });
  }, [savedWorkflows, searchTerm]);

  useEffect(() => {
    loadSavedWorkflows();
  }, []);

  const loadSavedWorkflows = async () => {
    try {
      setLoading(true);
      setError(null);
      const workflowManifestResponses = await listSavedWorkflows();
      setSavedWorkflows(workflowManifestResponses);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load workflows');
      console.error('Failed to load saved workflows:', err);
    } finally {
      setLoading(false);
    }
  };

  const handleStartWorkflowChat = async (workflow: Workflow, _workflowId: string) => {
    try {
      const newAgent = await startAgent({
        body: {
          working_dir: getInitialWorkingDir(),
          workflow,
        },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const session = newAgent.data;
      setView('pair', {
        disableAnimation: true,
        resumeSessionId: session.id,
      });
    } catch (error) {
      // A toast, not `setError`: that state is the LIST's load error, and
      // setting it replaced a list that had loaded fine with "Couldn't load
      // workflows", whose Try again reloads the list rather than the chat.
      console.error('Failed to start workflow chat:', error);
      toastError(startChatFailureNotice(error, { kept: false }));
    }
  };

  const handleStartWorkflowChatInNewWindow = (workflowId: string) => {
    try {
      window.electron.createChatWindow(
        undefined,
        getInitialWorkingDir(),
        undefined,
        undefined,
        'pair',
        workflowId
      );
    } catch (error) {
      console.error('Failed to open workflow in new window:', error);
    }
  };

  const handleDeleteWorkflow = async () => {
    if (!workflowToDelete || isDeletingWorkflow) return;
    const workflowManifest = workflowToDelete;
    setIsDeletingWorkflow(true);
    try {
      await deleteWorkflow({ body: { id: workflowManifest.id } });
      await loadSavedWorkflows();
      setWorkflowToDelete(null);
      toastSuccess({ title: copy.deleted(workflowManifest.workflow.title) });
    } catch (err) {
      // A toast with the reason, not `setError`: that state is the LIST's load
      // error, and it replaced a list that had loaded fine with "Couldn't load
      // workflows" over a delete that had merely been refused.
      console.error('Failed to delete workflow:', err);
      toastError({
        title: copy.couldNotDelete(workflowManifest.workflow.title),
        msg: errorMessage(err),
      });
      setWorkflowToDelete(null);
    } finally {
      setIsDeletingWorkflow(false);
    }
  };

  const handleEditWorkflow = (workflowManifest: WorkflowManifest) => {
    setSelectedWorkflow(workflowManifest);
    setShowEditor(true);
  };

  const handleEditorClose = (wasSaved?: boolean) => {
    setShowEditor(false);
    setSelectedWorkflow(null);
    if (wasSaved) {
      loadSavedWorkflows();
    }
  };

  const handleCopyDeeplink = async (workflowManifest: WorkflowManifest) => {
    try {
      const deeplink = await generateDeepLink(workflowManifest.workflow);
      await navigator.clipboard.writeText(deeplink);
      toastSuccess({ title: copy.linkCopied });
    } catch (error) {
      console.error('Failed to copy deeplink:', error);
      toastError({ title: copy.couldNotCopyLink, msg: errorMessage(error) });
    }
  };

  const handleCopyYaml = async (workflowManifest: WorkflowManifest) => {
    try {
      const response = await workflowToYaml({
        body: { workflow: workflowManifest.workflow },
        throwOnError: true,
      });

      if (!response.data?.yaml) {
        throw new Error('No YAML data returned from API');
      }

      await navigator.clipboard.writeText(response.data.yaml);
      toastSuccess({ title: copy.yamlCopied });
    } catch (error) {
      console.error('Failed to copy YAML:', error);
      toastError({ title: copy.couldNotCopyYaml, msg: errorMessage(error) });
    }
  };

  const handleExportFile = async (workflowManifest: WorkflowManifest) => {
    try {
      const response = await workflowToYaml({
        body: { workflow: workflowManifest.workflow },
        throwOnError: true,
      });

      if (!response.data?.yaml) {
        throw new Error('No YAML data returned from API');
      }

      const sanitizedTitle = (workflowManifest.workflow.title || 'workflow')
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '');

      const filename = `${sanitizedTitle}.yaml`;

      const result = await window.electron.showSaveDialog({
        title: copy.exportDialogTitle,
        defaultPath: filename,
        filters: [
          { name: copy.yamlFiles, extensions: ['yaml', 'yml'] },
          { name: copy.allFiles, extensions: ['*'] },
        ],
      });

      if (!result.canceled && result.filePath) {
        await window.electron.writeFile(result.filePath, response.data.yaml);
        toastSuccess({ title: copy.savedTo(result.filePath) });
      }
    } catch (error) {
      console.error('Failed to export workflow:', error);
      toastError({ title: copy.couldNotExport, msg: errorMessage(error) });
    }
  };

  const handleOpenScheduleDialog = (workflowManifest: WorkflowManifest) => {
    setScheduleWorkflowManifest(workflowManifest);
    setScheduleCron(workflowManifest.schedule_cron || '0 0 14 * * *');
    setShowScheduleDialog(true);
  };

  const closeScheduleDialog = () => {
    setShowScheduleDialog(false);
    setScheduleWorkflowManifest(null);
  };

  const handleSaveSchedule = async () => {
    if (!scheduleWorkflowManifest || isSavingSchedule) return;

    setIsSavingSchedule(true);
    try {
      // ⚠ Awaited AND read. This call used to send no proof and ignore the
      // answer, so a refused save — the daemon refuses a schedule whose work is
      // private to a caller it cannot believe (issue #56) — still toasted
      // "Schedule saved" over a workflow that was never scheduled.
      await scheduleWorkflowById(scheduleWorkflowManifest.id, scheduleCron);

      // Named here too. The confirmation is the last chance to notice that the
      // wrong row's button was pressed.
      toastSuccess({ title: copy.schedule.saved(scheduleWorkflowManifest.workflow.title) });

      closeScheduleDialog();
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to save schedule:', error);
      // A toast with the daemon's own words, and the dialog left open on the
      // schedule as it still is. Not `setError`: that state is the LIST's load
      // error, and it replaced a list that had loaded fine with "Couldn't load
      // workflows" (see `handleStartWorkflowChat`).
      toastError({
        title: copy.schedule.notSaved,
        msg: error instanceof Error ? error.message : 'Failed to save schedule',
      });
    } finally {
      setIsSavingSchedule(false);
    }
  };

  const handleRemoveSchedule = async () => {
    if (!scheduleWorkflowManifest || isSavingSchedule) return;

    setIsSavingSchedule(true);
    try {
      // ⚠ The worse half of the same defect: a refused removal toasted
      // "will no longer run automatically" while the schedule kept running.
      await scheduleWorkflowById(scheduleWorkflowManifest.id, null);

      toastSuccess({ title: copy.schedule.removed(scheduleWorkflowManifest.workflow.title) });

      closeScheduleDialog();
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to remove schedule:', error);
      toastError({
        title: copy.schedule.notRemoved,
        msg: error instanceof Error ? error.message : 'Failed to remove schedule',
      });
    } finally {
      setIsSavingSchedule(false);
    }
  };

  const handleOpenSlashCommandDialog = (workflowManifest: WorkflowManifest) => {
    setSlashCommandError('');
    setSlashCommandWorkflowManifest(workflowManifest);
    setSlashCommand(workflowManifest.slash_command || '');
    setShowSlashCommandDialog(true);
  };

  const closeSlashCommandDialog = () => {
    setShowSlashCommandDialog(false);
    setSlashCommandWorkflowManifest(null);
  };

  const refreshConfigAfterSlashCommandWrite = async () => {
    try {
      await refreshConfig();
    } catch (error) {
      console.error('Failed to refresh config after updating a workflow slash command:', error);
    }
  };

  const handleSaveSlashCommand = async () => {
    if (!slashCommandWorkflowManifest || isSavingSlashCommand) return;

    setIsSavingSlashCommand(true);
    setSlashCommandError('');
    try {
      await setWorkflowSlashCommand({
        throwOnError: true,
        body: {
          id: slashCommandWorkflowManifest.id,
          slash_command: slashCommand || null,
        },
      });
      await refreshConfigAfterSlashCommandWrite();

      toastSuccess({
        title: slashCommand ? copy.slash.saved(slashCommand) : copy.slash.removed,
      });

      closeSlashCommandDialog();
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to save slash command:', error);
      setSlashCommandError(
        typeof error === 'string' ? error : errorMessage(error, copy.slash.couldNotSave)
      );
    } finally {
      setIsSavingSlashCommand(false);
    }
  };

  const handleRemoveSlashCommand = async () => {
    if (!slashCommandWorkflowManifest || isSavingSlashCommand) return;

    setIsSavingSlashCommand(true);
    setSlashCommandError('');
    try {
      await setWorkflowSlashCommand({
        throwOnError: true,
        body: {
          id: slashCommandWorkflowManifest.id,
          slash_command: null,
        },
      });
      await refreshConfigAfterSlashCommandWrite();

      toastSuccess({ title: copy.slash.removed });

      closeSlashCommandDialog();
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to remove slash command:', error);
      setSlashCommandError(
        typeof error === 'string' ? error : errorMessage(error, copy.slash.couldNotRemove)
      );
    } finally {
      setIsSavingSlashCommand(false);
    }
  };

  const renderContent = () => {
    if (loading) {
      return (
        <div className="biorouter-list-shell br-rows-flat" aria-hidden>
          <WorkflowRowSkeleton />
          <WorkflowRowSkeleton />
          <WorkflowRowSkeleton />
        </div>
      );
    }

    if (error) {
      return (
        <EmptyState
          icon={AlertCircle}
          title={copy.loadFailedTitle}
          description={error}
          actions={
            <Button variant="link" onClick={loadSavedWorkflows}>
              {copy.tryAgain}
            </Button>
          }
        />
      );
    }

    // The band holds the view's one accent ("New workflow"), so the empty
    // state offers no second button: one short sentence and, at most, a link.
    if (savedWorkflows.length === 0) {
      return (
        <EmptyState
          icon={WorkflowIcon}
          title={copy.emptyTitle}
          description={copy.emptyDescription}
          actions={
            <Button variant="link" onClick={() => setShowImportDialog(true)}>
              {copy.importWorkflow}
            </Button>
          }
        />
      );
    }

    if (filteredWorkflows.length === 0 && searchTerm) {
      return (
        <EmptyState
          icon={WorkflowIcon}
          title={copy.noMatchTitle}
          description={copy.noMatchDescription}
          compact
        />
      );
    }

    return (
      <div className="biorouter-list-shell br-rows-flat">
        {filteredWorkflows.map((manifest: WorkflowManifest) => (
          <WorkflowRow
            key={manifest.id}
            manifest={manifest}
            onRun={() => handleStartWorkflowChat(manifest.workflow, manifest.id)}
            onRunInNewWindow={() => handleStartWorkflowChatInNewWindow(manifest.id)}
            onEdit={() => handleEditWorkflow(manifest)}
            onSlashCommand={() => handleOpenSlashCommandDialog(manifest)}
            onSchedule={() => handleOpenScheduleDialog(manifest)}
            onCopyLink={() => void handleCopyDeeplink(manifest)}
            onCopyYaml={() => void handleCopyYaml(manifest)}
            onExport={() => void handleExportFile(manifest)}
            onDelete={() => setWorkflowToDelete(manifest)}
          />
        ))}
      </div>
    );
  };

  return (
    <>
      <MainPanelLayout removeTopPadding>
        <div className="flex min-h-0 flex-1 flex-col">
          {/* The band (spec 3.10): the title with its help in an InfoTip, the
              filter (⌘F focuses it), Import as a ghost round icon, and "New
              workflow" as the view's one accent. */}
          <PageHeader
            title={copy.title}
            info={copy.info}
            actions={
              <>
                <FilterInput
                  value={searchTerm}
                  onValueChange={setSearchTerm}
                  label={copy.filterLabel}
                />
                <PageHeaderAction
                  icon={Upload}
                  label={copy.importWorkflow}
                  onClick={() => setShowImportDialog(true)}
                />
                <Button onClick={() => setShowCreateDialog(true)}>
                  <Plus />
                  {copy.newWorkflow}
                </Button>
              </>
            }
          />

          <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6 pt-2">
            <ScrollArea className="h-full">
              <div className="relative h-full pb-6">{renderContent()}</div>
            </ScrollArea>
          </ReadableContent>
        </div>
      </MainPanelLayout>

      {showEditor && selectedWorkflow && (
        <CreateEditWorkflowModal
          isOpen={showEditor}
          onClose={handleEditorClose}
          workflow={selectedWorkflow.workflow}
          workflowId={selectedWorkflow.id}
        />
      )}

      <ImportWorkflowForm
        isOpen={showImportDialog}
        onClose={() => setShowImportDialog(false)}
        onSuccess={loadSavedWorkflows}
      />

      {showCreateDialog && (
        <CreateEditWorkflowModal
          isOpen={showCreateDialog}
          onClose={() => {
            setShowCreateDialog(false);
            loadSavedWorkflows();
          }}
          isCreateMode={true}
        />
      )}

      {/* ⚠ The subtitle is not decoration. Every row offers the same
          "Schedule…" item, and QA scheduled the WRONG workflow on the first try
          with two rows on screen while the dialog named nothing. A dialog that
          acts on one of several similar things names the one it will act on,
          and the subtitle is also its accessible description. */}
      {showScheduleDialog && scheduleWorkflowManifest && (
        <ModalShell
          open={showScheduleDialog}
          onOpenChange={(open) => {
            if (!open && !isSavingSchedule) closeScheduleDialog();
          }}
          size="md"
          purpose="form"
          // Selects only, no text field: the dialog takes focus, not a closed trigger (V3-20).
          onOpenAutoFocus={focusDialogOnOpen}
          title={copy.schedule.title}
          subtitle={scheduleWorkflowManifest.workflow.title}
          footer={
            <>
              {scheduleWorkflowManifest.schedule_cron && (
                <Button
                  variant="ghost"
                  className="mr-auto text-text-danger"
                  onClick={handleRemoveSchedule}
                  disabled={isSavingSchedule}
                >
                  {isSavingSchedule ? copy.schedule.working : copy.schedule.remove}
                </Button>
              )}
              <Button variant="secondary" onClick={closeScheduleDialog} disabled={isSavingSchedule}>
                {copy.cancel}
              </Button>
              <Button onClick={handleSaveSchedule} disabled={!scheduleValid || isSavingSchedule}>
                {isSavingSchedule ? copy.schedule.saving : copy.schedule.save}
              </Button>
            </>
          }
        >
          <div className="pb-1">
            <CronPicker
              schedule={
                scheduleWorkflowManifest.schedule_cron
                  ? {
                      id: scheduleWorkflowManifest.id,
                      source: '',
                      cron: scheduleWorkflowManifest.schedule_cron,
                      last_run: null,
                      currently_running: false,
                      paused: false,
                    }
                  : null
              }
              onChange={setScheduleCron}
              isValid={setScheduleIsValid}
            />
          </div>
        </ModalShell>
      )}

      {showSlashCommandDialog && slashCommandWorkflowManifest && (
        <ModalShell
          open={showSlashCommandDialog}
          onOpenChange={(open) => {
            if (!open && !isSavingSlashCommand) closeSlashCommandDialog();
          }}
          size="md"
          purpose="form"
          title={copy.slash.title}
          subtitle={slashCommandWorkflowManifest.workflow.title}
          footer={
            <>
              {slashCommandWorkflowManifest.slash_command && (
                <Button
                  variant="ghost"
                  className="mr-auto text-text-danger"
                  onClick={handleRemoveSlashCommand}
                  disabled={isSavingSlashCommand}
                >
                  {isSavingSlashCommand ? copy.slash.working : copy.slash.remove}
                </Button>
              )}
              <Button
                variant="secondary"
                onClick={closeSlashCommandDialog}
                disabled={isSavingSlashCommand}
              >
                {copy.cancel}
              </Button>
              <Button onClick={handleSaveSlashCommand} disabled={isSavingSlashCommand}>
                {isSavingSlashCommand ? copy.slash.saving : copy.slash.save}
              </Button>
            </>
          }
        >
          {/* The Field recipe (label, 6px, control, one live line), written out
              because the control carries a leading "/" the Field cannot clone
              onto. The "/" is drawn INSIDE the field (V3-19). The one helper line is
              always there and follows what is typed; an error replaces it, in danger
              ink. */}
          <div className="br-form-field flex min-w-0 flex-col gap-1.5 pb-1" data-slot="field">
            <label htmlFor="workflow-slash-command" className="text-label text-text-default">
              {copy.slash.inputLabel}
            </label>
            <div className="br-workflow-slash-field">
              <span className="br-workflow-slash-glyph text-label" aria-hidden>
                /
              </span>
              <Input
                machine
                id="workflow-slash-command"
                type="text"
                value={slashCommand}
                onChange={(e) => setSlashCommand(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    void handleSaveSlashCommand();
                  }
                }}
                placeholder={copy.slash.placeholder}
                aria-invalid={slashCommandError ? true : undefined}
                aria-describedby="workflow-slash-command-help"
                className="br-workflow-slash-input"
              />
            </div>
            {slashCommandError ? (
              <p
                id="workflow-slash-command-help"
                role="alert"
                className="text-supporting text-text-danger"
              >
                {slashCommandError}
              </p>
            ) : (
              <p id="workflow-slash-command-help" className="text-supporting text-text-muted">
                {slashCommand.trim()
                  ? copy.slash.helper(slashCommand.trim())
                  : copy.slash.helperEmpty}
              </p>
            )}
          </div>
        </ModalShell>
      )}

      <ConfirmationModal
        isOpen={workflowToDelete !== null}
        title={copy.deleteTitle(workflowToDelete?.workflow.title ?? '')}
        message={copy.deleteMessage}
        confirmLabel={copy.deleteConfirm}
        cancelLabel={copy.cancel}
        confirmVariant="destructive"
        isSubmitting={isDeletingWorkflow}
        onConfirm={() => void handleDeleteWorkflow()}
        onCancel={() => setWorkflowToDelete(null)}
      />
    </>
  );
}

// ---------------------------------------------------------------------------
// One workflow, as a two-line row
// ---------------------------------------------------------------------------

interface WorkflowRowProps {
  manifest: WorkflowManifest;
  onRun: () => void;
  onRunInNewWindow: () => void;
  onEdit: () => void;
  onSlashCommand: () => void;
  onSchedule: () => void;
  onCopyLink: () => void;
  onCopyYaml: () => void;
  onExport: () => void;
  onDelete: () => void;
}

/**
 * One workflow (spec 3.10). Line 1 is the title and its state as neutral badges: the slash
 * command, a short schedule ("Daily 2 PM", the whole rule in its help) and Built-in. Line 2 is
 * the description, muted, one line. At rest the trailing edge shows only when it was modified
 * (never for a built-in, whose file is rewritten at each launch); on hover or focus that gives
 * way to Run and `⋯`, whose menu holds every other verb.
 * Right-click and Shift+F10 open the same menu, and a click on the row opens Edit.
 *
 * The state the two tinted buttons used to carry (a slash command exists, a schedule exists)
 * did not go away with the buttons: it is the badges.
 *
 * ⚠ Every text box down the column carries `min-w-0`: a flex item's `min-width` is `auto`,
 * which resolves to its min-content width, and `truncate` makes that the WHOLE string, so a
 * long title would push Run and `⋯` out of the row instead of truncating.
 */
export function WorkflowRow({
  manifest,
  onRun,
  onRunInNewWindow,
  onEdit,
  onSlashCommand,
  onSchedule,
  onCopyLink,
  onCopyYaml,
  onExport,
  onDelete,
}: WorkflowRowProps) {
  const { workflow, schedule_cron: scheduleCron, slash_command: slashCommand } = manifest;
  const title = workflow.title;
  const cronBadge = shortCron(scheduleCron);
  const builtIn = isBuiltinWorkflow(manifest.file_path);
  // A built-in is rewritten at every launch, so its `last_modified` is always "just now" and
  // says nothing about the workflow; its Built-in badge is the whole of what it has to say
  // (V3-22). Everything else shows when it was last edited.
  const modified = builtIn ? null : modifiedAt(manifest.last_modified);
  const modifiedText = rowTime(modified);
  const modifiedTitle = rowTimeTitle(modified);

  const menu: RowActionItem[] = [
    { label: copy.runInNewWindow, onSelect: onRunInNewWindow },
    { label: copy.edit, onSelect: onEdit },
    { label: copy.slashCommandItem, onSelect: onSlashCommand },
    { label: copy.scheduleItem, onSelect: onSchedule },
    { kind: 'separator' },
    { label: copy.copyLink, onSelect: onCopyLink },
    { label: copy.copyYaml, onSelect: onCopyYaml },
    { label: copy.exportToFile, onSelect: onExport },
    { kind: 'separator' },
    { label: copy.delete, onSelect: onDelete, destructive: true },
  ];

  return (
    <RowContextMenu items={menu}>
      <div
        className="biorouter-list-row flex cursor-pointer items-center gap-3"
        data-testid="workflow-row"
        onClick={(event) => {
          // React delivers clicks from portalled menus through the row too; only a click on
          // the row's own surface opens the editor.
          const row = event.currentTarget;
          const target = event.target;
          if (!(target instanceof Node) || !row.contains(target)) return;
          const control = target instanceof Element ? target.closest(ROW_CONTROL_SELECTOR) : null;
          if (control && control !== row && row.contains(control)) return;
          onEdit();
        }}
      >
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            {/* No `title` attribute: AppTooltipLayer turns it into a tooltip that repeats the
                title the row is already showing, and a Scheduler or Skills row shows none. The
                full name is in the editor, one click away. */}
            <h3 className="min-w-0 truncate text-label text-text-default">{title}</h3>
            {slashCommand && <Badge tone="neutral">/{slashCommand}</Badge>}
            {scheduleCron && cronBadge && (
              <InfoTip label={copy.scheduleBadgeLabel} help={cronSentence(scheduleCron)} asChild>
                <Badge tone="neutral" className="tabular-nums">
                  {cronBadge}
                </Badge>
              </InfoTip>
            )}
            {builtIn && <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />}
          </div>
          {workflow.description && (
            <p className="min-w-0 truncate text-supporting text-text-muted">
              {workflow.description}
            </p>
          )}
        </div>
        <RowActions
          primary={
            <IconAction
              icon={Play}
              label={copy.runNamed(title)}
              tooltip={copy.run}
              onSelect={onRun}
            />
          }
          menu={menu}
          menuLabel={copy.moreActionsNamed(title)}
          meta={
            modifiedText ? (
              <time
                className="tabular-nums"
                dateTime={modified?.toISOString()}
                title={modifiedTitle ? copy.modified(modifiedTitle) : undefined}
              >
                {modifiedText}
              </time>
            ) : undefined
          }
        />
      </div>
    </RowContextMenu>
  );
}

/** Loading is rows that are the shape of rows, on the list the loaded workflows land in. */
function WorkflowRowSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="mt-1 h-3 w-64" />
      </div>
    </div>
  );
}
