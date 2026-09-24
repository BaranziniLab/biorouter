import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from 'react';
import type { ProviderDetails } from '../../../api';
import { useNavigate } from 'react-router-dom';
import { AlertTriangle } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import type { ObservedRun } from '../crewApi';
import { channelName, channelNamesAcrossTeams, teamName } from '../identity';
import { useCrewErrorSlot } from '../state/CrewControllerContext';
import { agentCopy, LONG_TASK_CHARS, LONG_TASK_LINES, unknownOutcomeCopy } from './copy';
import { CrewModelPicker, ModelTierMarks } from './CrewModelPicker';
import { newestTaskIn } from './newestTask';
import {
  isAffiliationRefusal,
  knownInstitutions,
  modelDisplay,
  modelMismatch,
  runInstitution,
  usePanePresentation,
  workspaceInstitutionLabel,
} from './presentation';
import { useConfiguredModels, type ModelChoice } from './useConfiguredModels';
import './pane.css';

/** The Task field's height at rest; it grows with what is written (T-47). */
const TASK_MIN_ROWS = 6;

export interface AgentTaskPaneProps {
  /**
   * Scroll to and highlight a task's row in the timeline ("Show task in channel"). The pane
   * passes the viewer's newest task in this channel and closes itself. The highlight belongs to
   * whoever renders the timeline (its `highlightRunId`), so the layout wires it here through
   * `DetailsPane`'s `agent` slot, as it does for the sidebar's Agents section.
   */
  onShowTask?(run: ObservedRun): void;
  /** Layout only. */
  className?: string;
}

/**
 * Ask my agent (ui-redesign-spec, "Ask my agent"): start the viewer's own agent on a task that
 * reads this channel and posts in it.
 *
 * - The destination is always shown; it is the consent.
 * - **Task** has its own state, seeded once from the composer draft when the pane opens (L9). On
 *   success the Task clears, the composer draft clears only if it still equals that seed, and the
 *   pane closes. The daemon posts the task itself in the channel ("Task: …"), so the field says so,
 *   and says it again for a task long enough to be pasted data (T-24). It is at least six lines and
 *   grows with what is written (`.crew-agent-task`).
 * - **Model**: the app's default model as a summary with Change when it resolves to a configured
 *   provider, else the picker; "No models are set up." with Open Settings when nothing is
 *   configured (Crew bypasses provider onboarding). A model is named as the composer's model chip
 *   names it and wraps rather than ellipsizing. A private model the workspace's institution has
 *   not approved is explained before Start, which it disables, and the daemon's "resolved
 *   affiliation" refusal is reworded the same way (T-47).
 * - **Advanced** holds Also read (unmounted while closed, which keeps the unknown-outcome gate's
 *   checkbox the only one in the document, C9) and the remote folder line. With neither to offer
 *   it is not shown at all.
 * - Its errors are `pane:agent`'s. `DetailsPane` dismisses one when the pane leaves this mode, so
 *   a refusal does not reappear in the connection bar for a drawer that is gone (T-48).
 * - **Start my agent and allow posting here** is rendered unconditionally with a stable key, and
 *   its errors render in a fixed slot above it, so the node never remounts (C11).
 * - After `crew_start_outcome_unknown` the gate asks the person to inspect the previous task
 *   first. Start stays rendered and disabled (C12); **Start a new task** is enabled by the one
 *   checkbox and runs `form.reportValidity()` before a deliberate restart, which rotates the
 *   request id. The lock itself is module-scoped in the controller (C10). **Show task in channel**
 *   closes the pane and hands the viewer's newest task here to `onShowTask`, which the timeline
 *   answers by scrolling to and washing that task's row.
 *
 * React authorizes nothing here: the daemon checks privacy, the model's tier and institution, and
 * the posting grant when the task starts. Disabling Start for a model that cannot run here only
 * says early what the daemon would refuse; every other model still takes the request path.
 */
export function AgentTaskPane({ onShowTask, className }: AgentTaskPaneProps) {
  const { crew, snapshot, channel, team, verified, workspace } = usePanePresentation();
  const navigate = useNavigate();
  const [seed] = useState(() => crew.draft.body);
  const [task, setTask] = useState(seed);
  const models = useConfiguredModels();
  const [choice, setChoice] = useState<ModelChoice | null>(null);
  const [picking, setPicking] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [modelInvalid, setModelInvalid] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const showError = useCrewErrorSlot('pane:agent');
  const starting = useRef(false);
  const form = useRef<HTMLFormElement>(null);
  const picker = useRef<HTMLButtonElement>(null);
  const taskId = useId();
  const taskHintId = useId();
  const modelLabelId = useId();
  const modelErrorId = useId();
  const mismatchId = useId();
  const gateTitleId = useId();

  const { reportError, dismissError, error } = crew;
  useEffect(() => {
    if (models.failure) reportError(models.failure, 'pane:agent');
  }, [models.failure, reportError]);

  const selected = choice ?? models.defaults;
  const summary = !picking && choice === null && models.defaults !== null;
  const noModels = models.providers !== null && models.providers.length === 0;
  const selectedProvider = models.providers?.find((item) => item.name === selected?.provider);
  const unknown = crew.unknownRunDestination;
  const pending = crew.isPending('run.start');

  const otherChannels = useMemo(
    () => (snapshot?.channels ?? []).filter((item) => item.id !== channel?.id && !item.archived),
    [snapshot, channel]
  );
  const channelLabels = useMemo(
    () => channelNamesAcrossTeams(snapshot?.channels ?? [], snapshot?.teams ?? []),
    [snapshot]
  );
  const known = useMemo(() => knownInstitutions(models.providers), [models.providers]);
  const readsRestricted =
    channel?.classification === 'restricted' ||
    crew.contextChannels.some(
      (id) => snapshot?.channels.find((item) => item.id === id)?.classification === 'restricted'
    );

  if (!channel) return null;

  const here = channelName(channel);
  const institution = runInstitution({
    connection: crew.connection,
    snapshot,
    channel,
    contextChannels: crew.contextChannels,
    known,
  });
  const notApproved = (provider: ProviderDetails) =>
    modelMismatch(provider, institution) && institution
      ? agentCopy.notApproved(institution.label)
      : null;
  const shown = selected ? modelDisplay(selected, selectedProvider) : null;
  const shownName = shown ? agentCopy.modelChoice(shown.model, shown.provider) : '';
  const mismatch = modelMismatch(selectedProvider, institution);
  const mismatchText =
    mismatch && institution && shown
      ? mismatch.affiliation
        ? agentCopy.institutionMismatch(
            shown.model,
            mismatch.affiliation,
            workspace,
            institution.label
          )
        : agentCopy.institutionUnstated(shown.model, workspace, institution.label)
      : null;
  // The daemon's refusal in the pane's words: the same sentence when the pane can say who approved
  // the model, else what it does know.
  const errorText =
    error && isAffiliationRefusal(error.message)
      ? (mismatchText ??
        agentCopy.institutionRefused(
          shown?.model ?? agentCopy.model,
          institution?.label ?? workspaceInstitutionLabel(crew.connection, snapshot, known)
        ))
      : error?.message;
  const lines = task.split('\n').length;
  const longTask = lines > LONG_TASK_LINES || task.length > LONG_TASK_CHARS;
  const hasAdvanced =
    otherChannels.length > 0 ||
    crew.contextChannels.length > 0 ||
    Boolean(crew.connection?.remote_root);
  const unknownDestination =
    unknown ===
    `${crew.connection?.name ?? crew.connectionId} / ${team?.name ?? crew.teamId} / #${channel.name}`
      ? unknownOutcomeCopy.destination(here, teamName(team))
      : unknown;

  const submit = async (deliberateRestart: boolean) => {
    if (starting.current) return;
    if (!selected) {
      setModelInvalid(true);
      picker.current?.focus();
      return;
    }
    starting.current = true;
    try {
      const started = await crew.startOwnedRun({
        prompt: task,
        provider: selected.provider,
        model: selected.model,
        contextChannels: crew.contextChannels,
        deliberateRestart,
      });
      if (started) {
        setTask('');
        crew.clearBodyIfEquals(seed);
        crew.closePane();
      }
    } finally {
      starting.current = false;
    }
  };
  const onSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    void submit(false);
  };
  const showTask = () => {
    crew.setInspectedPriorRun(false);
    const run = newestTaskIn(crew.runs, crew.messages, channel.id);
    crew.closePane();
    if (run) onShowTask?.(run);
  };

  const startDisabled =
    pending ||
    Boolean(unknown) ||
    !verified ||
    channel.archived ||
    noModels ||
    mismatchText !== null;

  return (
    <form ref={form} onSubmit={onSubmit} className={cn('flex min-h-0 flex-1 flex-col', className)}>
      <div className="crew-pane-body flex flex-col gap-4 py-3">
        {unknown && (
          <section aria-labelledby={gateTitleId} className="flex flex-col gap-2">
            <Note tone="warning" role="alert" icon={AlertTriangle}>
              <p id={gateTitleId} className="font-medium">
                {unknownOutcomeCopy.title}
              </p>
              <p>{unknownOutcomeCopy.body(unknownDestination ?? here)}</p>
            </Note>
            <div className="flex flex-wrap gap-2">
              <Button type="button" variant="secondary" size="sm" onClick={showTask}>
                {unknownOutcomeCopy.showTask}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => {
                  crew.setInspectedPriorRun(false);
                  navigate('/sessions');
                }}
              >
                {unknownOutcomeCopy.openHistory}
              </Button>
            </div>
            <label className="flex items-start gap-2 text-supporting text-text-default">
              <Checkbox
                checked={crew.inspectedPriorRun}
                onChange={(event) => crew.setInspectedPriorRun(event.target.checked)}
              />
              <span className="pt-0.5">{unknownOutcomeCopy.checked}</span>
            </label>
            <div>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={!crew.inspectedPriorRun || pending || mismatchText !== null}
                onClick={() => {
                  if (form.current?.reportValidity()) void submit(true);
                }}
              >
                {unknownOutcomeCopy.restart}
              </Button>
            </div>
          </section>
        )}

        <p className="text-supporting text-text-muted">
          {agentCopy.destination(here, teamName(team), workspace)}
        </p>

        <div className="flex flex-col gap-1.5">
          <label htmlFor={taskId} className="text-label text-text-default">
            {agentCopy.task}
          </label>
          {/* Focus turns the border to the accent edge, as the chat composer's card does; the
              rule is authored in pane.css (`.crew-agent-task`), never a Tailwind variant (T-16). */}
          <textarea
            id={taskId}
            data-crew-pane-autofocus=""
            required
            rows={TASK_MIN_ROWS}
            value={task}
            placeholder={agentCopy.taskPlaceholder}
            aria-describedby={taskHintId}
            onChange={(event) => setTask(event.target.value)}
            className="crew-agent-task w-full resize-y rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 text-body text-text-default transition-[color,background-color,border-color,box-shadow] placeholder:text-text-muted hover:inset-ring-2 hover:inset-ring-border-emphasized/30"
          />
          <p id={taskHintId} className="text-supporting text-text-muted">
            {agentCopy.taskPosted(here)}
            {longTask && ` ${agentCopy.taskLong}`}
          </p>
        </div>

        <div className="flex flex-col gap-1.5">
          {summary && selected ? (
            // A group named "Model", so the summary answers to the same name as the picker.
            // It wraps rather than ellipsizing (T-47): the provider was the part a truncation cut.
            <div
              role="group"
              aria-labelledby={modelLabelId}
              className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1"
            >
              <span id={modelLabelId} className="text-label text-text-default">
                {agentCopy.model}
              </span>
              <span className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5 text-label text-text-default">
                <span className="min-w-0 break-words">{shownName}</span>
                <ModelTierMarks provider={selectedProvider} />
              </span>
              <Button
                type="button"
                variant="link"
                className="h-auto p-0 text-label"
                aria-label={agentCopy.modelChangeName}
                onClick={() => {
                  setChoice(models.defaults);
                  setPicking(true);
                  setPickerOpen(true);
                }}
              >
                {agentCopy.modelChange}
              </Button>
            </div>
          ) : noModels ? (
            <>
              <span id={modelLabelId} className="text-label text-text-default">
                {agentCopy.model}
              </span>
              <p className="flex flex-wrap items-center gap-x-2 text-supporting text-text-muted">
                {agentCopy.noModels}
                <Button
                  type="button"
                  variant="link"
                  className="h-auto p-0 text-supporting"
                  onClick={() => navigate('/settings', { state: { section: 'models' } })}
                >
                  {agentCopy.openSettings}
                </Button>
              </p>
            </>
          ) : (
            <>
              <span id={modelLabelId} className="text-label text-text-default">
                {agentCopy.model}
              </span>
              <CrewModelPicker
                ref={picker}
                providers={models.providers}
                provider={selected?.provider ?? ''}
                model={selected?.model ?? ''}
                labelledBy={modelLabelId}
                open={pickerOpen}
                onOpenChange={setPickerOpen}
                unavailableReason={notApproved}
                invalid={modelInvalid}
                describedBy={modelInvalid ? modelErrorId : undefined}
                onChange={(next) => {
                  setChoice(next);
                  setPicking(true);
                  setModelInvalid(false);
                  // A refusal of the model just replaced is no longer about the choice on screen.
                  if (error?.source === 'pane:agent' && isAffiliationRefusal(error.message)) {
                    dismissError();
                  }
                }}
              />
              {modelInvalid && (
                <p id={modelErrorId} className="text-supporting text-text-danger">
                  {agentCopy.modelRequired}
                </p>
              )}
            </>
          )}
        </div>

        {hasAdvanced && (
          <Disclosure
            open={advancedOpen}
            onOpenChange={setAdvancedOpen}
            summary={agentCopy.advancedSummary(crew.contextChannels.length)}
          >
            <div className="flex flex-col gap-3">
              {otherChannels.length > 0 && (
                <fieldset className="flex flex-col gap-1">
                  <legend className="mb-1 text-label text-text-default">
                    {agentCopy.alsoRead}
                  </legend>
                  {otherChannels.map((item) => (
                    <label
                      key={item.id}
                      className="flex min-w-0 items-center gap-2 text-label text-text-default"
                    >
                      <Checkbox
                        checked={crew.contextChannels.includes(item.id)}
                        onChange={(event) =>
                          crew.setContextChannels(
                            event.target.checked
                              ? [...crew.contextChannels, item.id]
                              : crew.contextChannels.filter((id) => id !== item.id)
                          )
                        }
                      />
                      <span className="min-w-0 truncate">
                        {channelLabels.get(item.id) ?? channelName(item)}
                      </span>
                    </label>
                  ))}
                </fieldset>
              )}
              {crew.connection?.remote_root && (
                <p className="text-supporting text-text-muted">
                  {crew.connection.remote_execution
                    ? agentCopy.folderExec(crew.connection.remote_root)
                    : agentCopy.folderRead(crew.connection.remote_root)}
                </p>
              )}
            </div>
          </Disclosure>
        )}

        {selectedProvider?.resolved_tier === 'public' && readsRestricted && (
          <Note tone="warning" icon={AlertTriangle}>
            <p>{agentCopy.publicHint}</p>
          </Note>
        )}
      </div>

      <div className="crew-pane-footer flex flex-col gap-2 py-3">
        <p className="text-supporting text-text-muted">{agentCopy.scope(here)}</p>
        {/* Before Start, which it disables: the pane says what the daemon would refuse (T-47). */}
        {mismatchText && (
          <Note tone="warning" icon={AlertTriangle}>
            <p id={mismatchId}>{mismatchText}</p>
          </Note>
        )}
        {/* The fixed error slot: its sibling below never moves or remounts (C11). */}
        <div>
          {showError && error && (
            <Note tone="danger" role="alert" icon={AlertTriangle}>
              <p>{errorText}</p>
            </Note>
          )}
        </div>
        <div className="flex justify-end">
          <Button
            key="start"
            type="submit"
            disabled={startDisabled}
            aria-describedby={mismatchText ? mismatchId : undefined}
          >
            {agentCopy.start}
          </Button>
        </div>
      </div>
    </form>
  );
}
