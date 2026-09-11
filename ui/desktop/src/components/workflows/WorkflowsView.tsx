import { useState, useEffect, useMemo } from 'react';
import { listSavedWorkflows, convertToLocaleDateString } from '../../workflow/workflow_management';
import {
  Edit,
  Trash2,
  Play,
  Calendar,
  AlertCircle,
  Link,
  Clock,
  Terminal,
  NewWindow,
  Share2,
  Copy,
  Download,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { ScrollArea } from '../ui/scroll-area';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Skeleton } from '../ui/skeleton';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { MODAL_SIZE } from '../ModalShell';
import { toastSuccess, toastError } from '../../toasts';
import {
  deleteWorkflow,
  WorkflowManifest,
  startAgent,
  scheduleWorkflow,
  setWorkflowSlashCommand,
  workflowToYaml,
} from '../../api';
import ImportWorkflowForm, { ImportWorkflowButton } from './ImportWorkflowForm';
import CreateEditWorkflowModal from './CreateEditWorkflowModal';
import { generateDeepLink, Workflow } from '../../workflow';
import { useNavigation } from '../../hooks/useNavigation';
import { CronPicker } from '../schedule/CronPicker';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '../ui/dialog';
import { SearchView } from '../conversation/SearchView';
import cronstrue from 'cronstrue';
import { getInitialWorkingDir } from '../../utils/workingDir';
import { startChatFailureNotice } from '../../utils/startChatFailure';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  DropdownMenuSeparator,
} from '../ui/dropdown-menu';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import BuiltInBadge from '../ui/BuiltInBadge';
import { BUILTIN_RECREATED_TITLE, isBuiltinWorkflow } from '../../utils/builtins';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { useConfig } from '../ConfigContext';
import { userActionHeaders } from '../../utils/userAction';

const WorkflowIcon = ENTITY_ICONS.workflow;

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
      toastSuccess({
        title: workflowManifest.workflow.title,
        msg: 'Workflow deleted',
      });
    } catch (err) {
      console.error('Failed to delete workflow:', err);
      const errorMsg = err instanceof Error ? err.message : 'Failed to delete workflow';
      setError(errorMsg);
    } finally {
      setIsDeletingWorkflow(false);
    }
  };

  const handleEditWorkflow = async (workflowManifest: WorkflowManifest) => {
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
      toastSuccess({
        title: 'Deeplink copied',
        msg: 'Workflow deeplink has been copied to clipboard',
      });
    } catch (error) {
      console.error('Failed to copy deeplink:', error);
      toastError({
        title: 'Copy failed',
        msg: 'Failed to copy deeplink to clipboard',
      });
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
      toastSuccess({
        title: 'YAML copied',
        msg: 'Workflow YAML has been copied to clipboard',
      });
    } catch (error) {
      console.error('Failed to copy YAML:', error);
      toastError({
        title: 'Copy failed',
        msg: 'Failed to copy workflow YAML to clipboard',
      });
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
        title: 'Export workflow',
        defaultPath: filename,
        filters: [
          { name: 'YAML Files', extensions: ['yaml', 'yml'] },
          { name: 'All Files', extensions: ['*'] },
        ],
      });

      if (!result.canceled && result.filePath) {
        await window.electron.writeFile(result.filePath, response.data.yaml);
        toastSuccess({
          title: 'Workflow exported',
          msg: `Workflow saved to ${result.filePath}`,
        });
      }
    } catch (error) {
      console.error('Failed to export workflow:', error);
      toastError({
        title: 'Export failed',
        msg: 'Failed to export workflow to file',
      });
    }
  };

  const handleOpenScheduleDialog = (workflowManifest: WorkflowManifest) => {
    setScheduleWorkflowManifest(workflowManifest);
    setScheduleCron(workflowManifest.schedule_cron || '0 0 14 * * *');
    setShowScheduleDialog(true);
  };

  const handleSaveSchedule = async () => {
    if (!scheduleWorkflowManifest || isSavingSchedule) return;

    setIsSavingSchedule(true);
    try {
      await scheduleWorkflow({
        body: {
          id: scheduleWorkflowManifest.id,
          cron_schedule: scheduleCron,
        },
      });

      toastSuccess({
        title: 'Schedule saved',
        msg: `Workflow will run ${getReadableCron(scheduleCron)}`,
      });

      setShowScheduleDialog(false);
      setScheduleWorkflowManifest(null);
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to save schedule:', error);
      const errorMsg = error instanceof Error ? error.message : 'Failed to save schedule';
      setError(errorMsg);
    } finally {
      setIsSavingSchedule(false);
    }
  };

  const handleRemoveSchedule = async () => {
    if (!scheduleWorkflowManifest || isSavingSchedule) return;

    setIsSavingSchedule(true);
    try {
      await scheduleWorkflow({
        body: {
          id: scheduleWorkflowManifest.id,
          cron_schedule: null,
        },
      });

      toastSuccess({
        title: 'Schedule removed',
        msg: 'Workflow will no longer run automatically',
      });

      setShowScheduleDialog(false);
      setScheduleWorkflowManifest(null);
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to remove schedule:', error);
      const errorMsg = error instanceof Error ? error.message : 'Failed to remove schedule';
      setError(errorMsg);
    } finally {
      setIsSavingSchedule(false);
    }
  };

  const handleOpenSlashCommandDialog = (workflowManifest: WorkflowManifest) => {
    setSlashCommandWorkflowManifest(workflowManifest);
    setSlashCommand(workflowManifest.slash_command || '');
    setShowSlashCommandDialog(true);
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
    try {
      await setWorkflowSlashCommand({
        body: {
          id: slashCommandWorkflowManifest.id,
          slash_command: slashCommand || null,
        },
      });
      await refreshConfigAfterSlashCommandWrite();

      toastSuccess({
        title: 'Slash command saved',
        msg: slashCommand ? `Use /${slashCommand} to run this workflow` : 'Slash command removed',
      });

      setShowSlashCommandDialog(false);
      setSlashCommandWorkflowManifest(null);
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to save slash command:', error);
      const errorMsg = error instanceof Error ? error.message : 'Failed to save slash command';
      setError(errorMsg);
    } finally {
      setIsSavingSlashCommand(false);
    }
  };

  const handleRemoveSlashCommand = async () => {
    if (!slashCommandWorkflowManifest || isSavingSlashCommand) return;

    setIsSavingSlashCommand(true);
    try {
      await setWorkflowSlashCommand({
        body: {
          id: slashCommandWorkflowManifest.id,
          slash_command: null,
        },
      });
      await refreshConfigAfterSlashCommandWrite();

      toastSuccess({
        title: 'Slash command removed',
        msg: 'Workflow slash command has been removed',
      });

      setShowSlashCommandDialog(false);
      setSlashCommandWorkflowManifest(null);
      await loadSavedWorkflows();
    } catch (error) {
      console.error('Failed to remove slash command:', error);
      const errorMsg = error instanceof Error ? error.message : 'Failed to remove slash command';
      setError(errorMsg);
    } finally {
      setIsSavingSlashCommand(false);
    }
  };

  const getReadableCron = (cron: string): string => {
    try {
      const cronWithoutSeconds = cron.split(' ').slice(1).join(' ');
      return cronstrue.toString(cronWithoutSeconds).toLowerCase();
    } catch {
      return cron;
    }
  };

  const WorkflowItem = ({
    workflowManifestResponse,
    workflowManifestResponse: {
      workflow,
      last_modified: lastModified,
      schedule_cron,
      slash_command,
    },
  }: {
    workflowManifestResponse: WorkflowManifest;
  }) => (
    <div className="biorouter-list-row group px-3 py-3">
      {/*
       * The row has to survive the chat measure, and the arithmetic is tight:
       * 760px column − 48 (the body's `px-6`) = 712, − 24 (this row's `px-3`) =
       * 688 for the row, − 248 for the seven 32px actions and their six 4px gaps
       * − 12 for the `gap-3` between the two halves = 428px of text.
       *
       * So every FLEX ITEM holding text carries `min-w-0` and the action cluster
       * carries `shrink-0`. A flex item's default `min-width: auto` resolves to
       * its min-content width, and `truncate` is `white-space: nowrap`, whose
       * min-content width is the WHOLE string — so without `min-w-0` a long
       * title does not truncate, it pushes the actions out of the row.
       * `break-words` is not the fix either: it lowers the min-content width of
       * a box that WRAPS, and this one does not.
       *
       * ⚠ `min-w-0` is written ONLY on the real flex items. The three flex
       * CONTAINERS here — this outer row, the title line and the metadata line —
       * are themselves block-level boxes, and `min-width: auto` computes to 0
       * outside a flex or grid item, so a `min-w-0` on any of them would be one
       * more class that reads as intent and does nothing (vocabulary rule 4).
       * `flex-1`'s box below is the exception that proves it: that one IS a flex
       * item, of this row, and so it does carry one.
       *
       * The title's old `max-w-[50vw]` was a second measure keyed to the
       * VIEWPORT rather than to the pane: at 1440px it resolved to 720px — wider
       * than the 428 it was meant to fit inside — so it capped nothing here and
       * clipped arbitrarily in a narrow pane.
       */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h3 className="min-w-0 truncate text-label text-text-default" title={workflow.title}>
              {workflow.title}
            </h3>
            {/* No `shrink-0` wrapper: `Badge` already carries `flex-shrink-0`,
                and restating it here would be a second place for the two to
                drift (vocabulary V8 — reuse the primitive, do not re-declare
                what it already says). */}
            {isBuiltinWorkflow(workflowManifestResponse.file_path) && (
              <BuiltInBadge title={BUILTIN_RECREATED_TITLE} />
            )}
          </div>
          <p className="mt-0.5 line-clamp-1 text-supporting text-text-muted">
            {workflow.description}
          </p>
          {/* Wraps rather than overflows: the readable cron is a whole sentence
              ("at 02:00 PM every day"), so in a narrow pane it takes its own
              line instead of pushing the date off the row. */}
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-supporting text-text-subtle">
            <span className="flex shrink-0 items-center gap-1">
              <Calendar className="h-3 w-3 shrink-0" />
              {convertToLocaleDateString(lastModified)}
            </span>
            {schedule_cron && (
              // Two nested flex contexts, so `min-w-0` twice: once so this span
              // can shrink inside the metadata line, and once so the truncating
              // text can shrink inside this span. Either one alone leaves the
              // whole cron sentence at its min-content width.
              <span className="flex min-w-0 items-center gap-1 text-text-info">
                <Clock className="h-3 w-3 shrink-0" />
                <span className="min-w-0 truncate">Runs {getReadableCron(schedule_cron)}</span>
              </span>
            )}
            {slash_command && (
              <span className="min-w-0 truncate text-text-info">/{slash_command}</span>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
          <Button
            onClick={(e) => {
              e.stopPropagation();
              handleOpenSlashCommandDialog(workflowManifestResponse);
            }}
            variant="ghost"
            shape="round"
            /*
             * ⚠ State, not emphasis. This was `default` when a slash command
             * exists, so the state was drawn as a solid accent fill. Flattening
             * every row action to ghost would have DELETED that signal, which is
             * a functional regression rather than a style change.
             *
             * `tint-selected` is the system's answer for exactly this case:
             * selection is a state the app sets, not a user interaction. Paired
             * with `tint-interactive` because hover alone (5%) is lighter than
             * the selected wash (14%) and would visibly un-select on hover.
             *
             * The Run action beside it has since become a ghost like the rest
             * (operator decision: a CTA does not live inside a list row), so
             * this tint and its schedule twin are now the ONLY non-ghost
             * treatments left in the cluster — which is the point. They are the
             * two that say something about the workflow rather than about what
             * the button does, and that is what earns them the fill.
             */
            className={slash_command ? 'tint-selected tint-interactive' : undefined}
            title={slash_command ? 'Edit slash command' : 'Add slash command'}
          >
            <Terminal />
          </Button>
          <Button
            onClick={(e) => {
              e.stopPropagation();
              handleStartWorkflowChat(workflow, workflowManifestResponse.id);
            }}
            variant="ghost"
            shape="round"
            title="Use workflow"
          >
            <Play />
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                onClick={(e) => e.stopPropagation()}
                variant="ghost"
                shape="round"
                title="Launch workflow"
              >
                <NewWindow />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
              <DropdownMenuItem
                onClick={() => handleStartWorkflowChatInNewWindow(workflowManifestResponse.id)}
              >
                <NewWindow />
                Open in new window
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            onClick={(e) => {
              e.stopPropagation();
              handleEditWorkflow(workflowManifestResponse);
            }}
            variant="ghost"
            shape="round"
            title="Edit workflow"
          >
            <Edit />
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                onClick={(e) => e.stopPropagation()}
                variant="ghost"
                shape="round"
                title="Share workflow"
              >
                <Share2 />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
              <DropdownMenuItem onClick={() => handleCopyDeeplink(workflowManifestResponse)}>
                <Link />
                Copy deeplink
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => handleCopyYaml(workflowManifestResponse)}>
                <Copy />
                Copy YAML
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onClick={() => handleExportFile(workflowManifestResponse)}>
                <Download />
                Export to file
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            onClick={(e) => {
              e.stopPropagation();
              handleOpenScheduleDialog(workflowManifestResponse);
            }}
            variant="ghost"
            shape="round"
            // Same state-as-tint treatment as the slash-command button above.
            className={schedule_cron ? 'tint-selected tint-interactive' : undefined}
            title={schedule_cron ? 'Edit schedule' : 'Add schedule'}
          >
            <Clock />
          </Button>
          <Button
            onClick={(e) => {
              e.stopPropagation();
              setWorkflowToDelete(workflowManifestResponse);
            }}
            variant="ghost"
            shape="round"
            className="text-text-danger"
            title="Delete workflow"
          >
            <Trash2 />
          </Button>
        </div>
      </div>
    </div>
  );

  /**
   * Loading is rows that are the shape of rows, on the same hairline list the
   * loaded workflows land in — the shape `ScheduleRowSkeleton` uses next door.
   *
   * The three action placeholders this replaced were the wrong count (five
   * blocks for seven buttons) and the wrong promise: the actions are
   * `sm:opacity-0` until the row is hovered, so a skeleton that draws them
   * shows a cluster the loaded row will not. The padding is the ROW's `px-3
   * py-3`, not the `py-4` it used to carry, so the list does not step when the
   * placeholders are swapped for rows.
   */
  const WorkflowRowSkeleton = () => (
    <div className="biorouter-list-row px-3 py-3">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="mt-2 h-3 w-64" />
        <Skeleton className="mt-2 h-3 w-32" />
      </div>
    </div>
  );

  const renderContent = () => {
    if (loading) {
      return (
        <div className="biorouter-list-shell" aria-hidden>
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
          title="Couldn’t load workflows"
          description={error}
          actions={<Button onClick={loadSavedWorkflows}>Try again</Button>}
        />
      );
    }

    if (savedWorkflows.length === 0) {
      return (
        <EmptyState
          icon={WorkflowIcon}
          title="No workflows yet"
          description="Create a reusable workflow here, save one from a chat, or import an existing workflow."
          actions={
            <>
              <Button onClick={() => setShowCreateDialog(true)}>
                <WorkflowIcon />
                Create workflow
              </Button>
              <Button onClick={() => setShowImportDialog(true)} variant="outline">
                Import workflow
              </Button>
            </>
          }
        />
      );
    }

    if (filteredWorkflows.length === 0 && searchTerm) {
      return (
        <EmptyState
          icon={WorkflowIcon}
          title="No matching workflows"
          description="Try a different title, description, or slash command."
          compact
        />
      );
    }

    return (
      <div className="biorouter-list-shell">
        {filteredWorkflows.map((workflowManifestResponse: WorkflowManifest) => (
          <WorkflowItem
            key={workflowManifestResponse.id}
            workflowManifestResponse={workflowManifestResponse}
          />
        ))}
      </div>
    );
  };

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          {/* The shared header (Layout/PageHeader.tsx), not a ninth copy of the
              same eleven lines. It owns the full-bleed hairline, the chat
              measure, the `text-secondary` description and the control strip
              the actions sit in — so the two Buttons below carry variant and
              nothing else. */}
          <PageHeader
            title="Workflows"
            description={`View and manage your saved workflows to quickly start new chats with predefined configurations. ${getSearchShortcutText()} to search.`}
            actions={
              <>
                <Button onClick={() => setShowCreateDialog(true)}>
                  <WorkflowIcon />
                  Create workflow
                </Button>
                <ImportWorkflowButton onClick={() => setShowImportDialog(true)} />
              </>
            }
          />

          {/* The body's column carries the SAME size as the header's, or the
              step between them is visible along the full-bleed hairline they
              share. */}
          <ReadableContent size="chat" className="flex-1 min-h-0 relative px-6 pt-6">
            <ScrollArea className="h-full">
              <SearchView
                onSearch={(term) => setSearchTerm(term)}
                placeholder="Search workflows..."
              >
                <div className="h-full relative pb-8">{renderContent()}</div>
              </SearchView>
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

      {showScheduleDialog && scheduleWorkflowManifest && (
        <Dialog
          open={showScheduleDialog}
          onOpenChange={(open) => !isSavingSchedule && setShowScheduleDialog(open)}
        >
          {/* ⚠ `max-w-md` did nothing above 640px. `DialogContent`'s base already
              ends in `sm:max-w-lg`, and an UNPREFIXED `max-w-md` does not merge
              with a `sm:`-prefixed class — so the dialog was 512px on every
              desktop window and 448px only below the breakpoint. `MODAL_SIZE`
              is the ladder (V8: never a pixel literal for a dialog width), and
              its rungs are `sm:`-prefixed for exactly that reason. */}
          <DialogContent
            aria-describedby={undefined}
            dismissible={!isSavingSchedule}
            className={MODAL_SIZE.md}
          >
            <DialogHeader>
              <DialogTitle>
                {scheduleWorkflowManifest.schedule_cron ? 'Edit' : 'Add'} schedule
              </DialogTitle>
            </DialogHeader>
            <div className="space-y-4">
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
              <DialogFooter>
                {scheduleWorkflowManifest.schedule_cron && (
                  <Button
                    variant="outline"
                    onClick={handleRemoveSchedule}
                    disabled={isSavingSchedule}
                  >
                    {isSavingSchedule ? 'Working…' : 'Remove schedule'}
                  </Button>
                )}
                <Button
                  variant="outline"
                  onClick={() => setShowScheduleDialog(false)}
                  disabled={isSavingSchedule}
                >
                  Cancel
                </Button>
                <Button onClick={handleSaveSchedule} disabled={!scheduleValid || isSavingSchedule}>
                  {isSavingSchedule ? 'Saving…' : 'Save'}
                </Button>
              </DialogFooter>
            </div>
          </DialogContent>
        </Dialog>
      )}

      {showSlashCommandDialog && slashCommandWorkflowManifest && (
        <Dialog
          open={showSlashCommandDialog}
          onOpenChange={(open) => !isSavingSlashCommand && setShowSlashCommandDialog(open)}
        >
          {/* Same ladder, same reason as the schedule dialog above. */}
          <DialogContent dismissible={!isSavingSlashCommand} className={MODAL_SIZE.md}>
            <DialogHeader>
              <DialogTitle>Slash command</DialogTitle>
            </DialogHeader>
            <div className="space-y-4">
              <div>
                {/* `DialogDescription` already carries `text-body
                    text-text-muted`; restating them here is the drift V6 bans,
                    so only the layout margin stays. */}
                <DialogDescription className="mb-3">
                  Set a slash command to quickly run this workflow from any chat
                </DialogDescription>
                <div className="flex items-center gap-2">
                  <span className="text-label text-text-muted">/</span>
                  {/* The `Input` primitive, not a fifth hand-rolled field: it is
                      the 32px md rung every other control in the app sits on,
                      and it owns the `--border-emphasized` edge and the global
                      focus surface the hand-rolled one had neither of. */}
                  <Input
                    type="text"
                    value={slashCommand}
                    onChange={(e) => setSlashCommand(e.target.value)}
                    placeholder="command-name"
                    className="flex-1"
                  />
                </div>
                {slashCommand && (
                  <p className="mt-2 text-supporting text-text-muted">
                    Use /{slashCommand} in any chat to run this workflow
                  </p>
                )}
              </div>

              <DialogFooter>
                {slashCommandWorkflowManifest.slash_command && (
                  <Button
                    variant="outline"
                    onClick={handleRemoveSlashCommand}
                    disabled={isSavingSlashCommand}
                  >
                    {isSavingSlashCommand ? 'Working…' : 'Remove'}
                  </Button>
                )}
                <Button
                  variant="outline"
                  onClick={() => setShowSlashCommandDialog(false)}
                  disabled={isSavingSlashCommand}
                >
                  Cancel
                </Button>
                <Button onClick={handleSaveSlashCommand} disabled={isSavingSlashCommand}>
                  {isSavingSlashCommand ? 'Saving…' : 'Save'}
                </Button>
              </DialogFooter>
            </div>
          </DialogContent>
        </Dialog>
      )}

      <ConfirmationModal
        isOpen={workflowToDelete !== null}
        title={`Delete "${workflowToDelete?.workflow.title ?? ''}"?`}
        message="This permanently removes the workflow file. This action cannot be undone."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={isDeletingWorkflow}
        onConfirm={() => void handleDeleteWorkflow()}
        onCancel={() => setWorkflowToDelete(null)}
      />
    </>
  );
}
