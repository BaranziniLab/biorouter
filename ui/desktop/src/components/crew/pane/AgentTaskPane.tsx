import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from 'react';
import type { ProviderDetails } from '../../../api';
import { useNavigate } from 'react-router-dom';
import { AlertTriangle, Info, LoaderCircle } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import type { CrewMessage, ObservedRun } from '../crewApi';
import { channelName, channelNamesAcrossTeams, teamName } from '../identity';
import { HISTORY_PAGE_SIZE, reachesChannelStart } from '../timeline/groupMessages';
import { useCrewErrorSlot } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';
import { agentCopy, LONG_TASK_CHARS, LONG_TASK_LINES, unknownOutcomeCopy } from './copy';
import { CrewModelPicker, ModelTierMarks } from './CrewModelPicker';
import { newestTaskIn } from './newestTask';
import {
  fileBaseName,
  isAffiliationRefusal,
  knownInstitutions,
  mentionedFileNames,
  modelDisplay,
  modelMismatch,
  protectedRunContext,
  runInstitution,
  sameNamedFiles,
  sharedWhen,
  unsharedFileNames,
  usePanePresentation,
  workspaceInstitutionLabel,
  type SharedFile,
} from './presentation';
import { useConfiguredModels, type ModelChoice } from './useConfiguredModels';
import './pane.css';

/** The Task field's height at rest; it grows with what is written (T-47). */
const TASK_MIN_ROWS = 6;

type SharedItem = {
  kind: 'attachment' | 'reference';
  id: string;
  /** The newest sharing message's `created_at`, and its place in the loaded list. */
  sharedAt: number;
  order: number;
};

/**
 * The files and server paths shared in these messages: the Files tab's "In this channel". The same
 * file shared in two messages is one item, dated by the newer message.
 */
function sharedItems(
  messages: readonly CrewMessage[],
  channelId: string | undefined
): SharedItem[] {
  const byKey = new Map<string, SharedItem>();
  const items: SharedItem[] = [];
  messages.forEach((message, order) => {
    if (message.channel_id !== channelId) return;
    const sharedAt = Number.isFinite(message.created_at) ? message.created_at : 0;
    const add = (kind: SharedItem['kind'], id: string) => {
      const seen = byKey.get(`${kind}:${id}`);
      if (seen) {
        if (sharedAt >= seen.sharedAt) {
          seen.sharedAt = sharedAt;
          seen.order = order;
        }
        return;
      }
      const item: SharedItem = { kind, id, sharedAt, order };
      byKey.set(`${kind}:${id}`, item);
      items.push(item);
    };
    for (const id of message.attachments ?? []) add('attachment', id);
    for (const id of message.references ?? []) add('reference', id);
  });
  return items;
}

/**
 * The files shared in this channel — read from its loaded messages, the source the Files tab lists
 * — each with every name it answers to and when it was last shared, or `null` while any name is
 * unknown: until the messages have loaded, while a name is being fetched, or when one could not
 * be (Q2-15). Also `null` whenever the loaded messages may not be the whole channel, since a file
 * shared in a message that is not loaded would then be warned about as unshared (or left out of a
 * count of same-named files, Q3-02): while the opening backlog is still arriving, while an older
 * page is shown instead of the live tail, and once the list holds a full page (the channel may go
 * further back). Names are fetched only while `wanted` (the task names a file), once per file,
 * with `blob.status` and `reference.get` as the Files tab's rows fetch them. A reference answers to
 * its label and to its path's file name.
 *
 * Display only: it decides whether the pane warns or notes, and nothing else.
 */
function useSharedFiles(
  crew: CrewController,
  channelId: string | undefined,
  wanted: boolean
): readonly SharedFile[] | null {
  const { messages, messagesLoaded, connectionId, request } = crew;
  const items = useMemo(() => sharedItems(messages, channelId), [messages, channelId]);
  const key = `${connectionId}\n${items.map((item) => `${item.kind}:${item.id}`).join('\n')}`;
  const cache = useRef(new Map<string, readonly string[]>());
  const [resolved, setResolved] = useState<{
    key: string;
    names: (readonly string[])[] | null;
  } | null>(null);
  const pageSize =
    typeof crew.pageSize === 'number' && crew.pageSize > 0 ? crew.pageSize : HISTORY_PAGE_SIZE;
  const wholeChannel =
    crew.historyBefore === null &&
    crew.backlogComplete !== false &&
    reachesChannelStart(messages, pageSize);
  const ready = wanted && messagesLoaded && wholeChannel;

  useEffect(() => {
    if (!ready) return;
    const controller = new AbortController();
    let active = true;
    void Promise.all(
      items.map(async (item): Promise<readonly string[] | null> => {
        const cacheKey = `${connectionId}\n${item.kind}:${item.id}`;
        const cached = cache.current.get(cacheKey);
        if (cached) return cached;
        try {
          let names: string[];
          if (item.kind === 'attachment') {
            const blob = await request<{ name?: unknown }>(
              'blob.status',
              { blob_id: item.id },
              { signal: controller.signal }
            );
            names = [blob?.name].filter((name): name is string => typeof name === 'string');
          } else {
            const reference = await request<{ label?: unknown; path?: unknown }>(
              'reference.get',
              { reference_id: item.id },
              { signal: controller.signal }
            );
            names = [
              reference?.label,
              typeof reference?.path === 'string' ? fileBaseName(reference.path) : null,
            ].filter((name): name is string => typeof name === 'string' && name !== '');
          }
          cache.current.set(cacheKey, names);
          return names;
        } catch {
          return null;
        }
      })
    ).then((results) => {
      if (!active) return;
      setResolved({
        key,
        names: results.some((names) => names === null) ? null : results.map((names) => names ?? []),
      });
    });
    return () => {
      active = false;
      controller.abort();
    };
    // `key` stands for `items` and the connection: the same files are the same lookup.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, key, request]);

  const names = ready && resolved?.key === key ? resolved.names : null;
  // `sharedAt` is read from `items` as they are now, not from the lookup: a file shared again
  // keeps its key (and its names) but is dated by the newer message.
  return useMemo(
    () =>
      names === null
        ? null
        : items.map((item, index) => ({
            names: names[index] ?? [],
            sharedAt: item.sharedAt,
            order: item.order,
          })),
    [names, items]
  );
}

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
 *   and says it again for a task long enough to be pasted data (T-24). It is at least six lines,
 *   grows with what is written and has no resize grip (`.crew-agent-task`, Q2-30).
 * - A task that names a file (`counts.csv`) no message in the channel shares gets a warning before
 *   Start: "No file named … is shared in #…". It does not disable Start, and it is said only while
 *   every message in the channel is loaded (Q2-15). A task that names a file two or more different
 *   shared files carry gets a note, also before Start and also not a gate: "2 files named … are
 *   shared in #…. Your agent will use the newest one, shared at 1:55 AM." — the daemon tells the
 *   agent to do exactly that and to name the file it used (Q3-02). The Task field does not
 *   spell-check: file names and sample IDs are not words (Q3-34).
 * - **Model**: the app's default model as a summary with Change when it resolves to a configured
 *   provider, else the picker; "No models are set up." with Open Settings when nothing is
 *   configured (Crew bypasses provider onboarding). A model is named as the composer's model chip
 *   names it and wraps rather than ellipsizing, followed by one worded mark, "🔒 Private · UCSF"
 *   (`ModelTierMarks`, Q2-67). A private model the workspace's institution has not approved is
 *   explained before Start, which it disables, and the daemon's "resolved affiliation" refusal is
 *   reworded the same way (T-47).
 * - **Advanced** holds Also read (unmounted while closed, which keeps the unknown-outcome gate's
 *   checkbox the only one in the document, C9) and the remote folder line; closed, it says what it
 *   holds: "Reads only #general" or "Also reads #methods". With neither to offer it is not shown
 *   at all.
 * - Its errors are `pane:agent`'s. `DetailsPane` dismisses one when the pane leaves this mode, so
 *   a refusal does not reappear in the connection bar for a drawer that is gone (T-48).
 * - **Start my agent and allow posting here** is rendered unconditionally with a stable key, and
 *   its errors render in a fixed slot above it, so the node never remounts (C11). From the click
 *   until the pane closes it reads "Starting…" beside a spinner (Q2-67).
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
  // From the click until the pane closes, Start reads "Starting…" (Q2-67). A failure or an
  // unknown outcome gives Start its words back; reopening the pane starts afresh.
  const [submitting, setSubmitting] = useState(false);
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
  const fileWarningId = useId();
  const sameNameId = useId();

  const { reportError, dismissError, error } = crew;
  useEffect(() => {
    if (models.failure) reportError(models.failure, 'pane:agent');
  }, [models.failure, reportError]);

  const agentOpen = crew.ui.pane?.mode === 'agent';
  useEffect(() => {
    if (agentOpen) setSubmitting(false);
  }, [agentOpen]);

  const mentioned = useMemo(() => mentionedFileNames(task), [task]);
  const sharedFiles = useSharedFiles(crew, channel?.id, mentioned.length > 0);

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
  const privateOnly = protectedRunContext({
    connection: crew.connection,
    snapshot,
    channel,
    contextChannels: crew.contextChannels,
  });
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
  const folderText = crew.connection?.remote_root
    ? crew.connection.remote_execution
      ? agentCopy.folderExec(crew.connection.remote_root)
      : agentCopy.folderRead(crew.connection.remote_root)
    : null;
  const alsoReads = crew.contextChannels
    .filter((id) => id !== channel.id)
    .map((id) => {
      const item = snapshot?.channels.find((candidate) => candidate.id === id);
      return item ? (channelLabels.get(id) ?? channelName(item)) : null;
    })
    .filter((label): label is string => label !== null);
  // Only once every shared file's name is known: a warning must not appear for a file that is
  // still loading, and "No file named …" is said only when it is true of the whole channel.
  const unshared =
    sharedFiles === null
      ? []
      : unsharedFileNames(
          mentioned,
          sharedFiles.flatMap((file) => file.names)
        );
  // Two or more different files by a name the task mentions (Q3-02): the agent is told to use the
  // newest, and the pane says which one that is before Start. Only once every name is known, as
  // above, so the count is the whole channel's.
  const sameNamed = sharedFiles === null ? [] : sameNamedFiles(mentioned, sharedFiles);
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
    setSubmitting(true);
    let started = false;
    try {
      started = await crew.startOwnedRun({
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
      if (!started) setSubmitting(false);
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
    submitting ||
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
            // File names and sample IDs are not words: the spell checker underlined
            // `gina-assay.csv` and `15b` in every task (Q3-34).
            spellCheck={false}
            onChange={(event) => setTask(event.target.value)}
            className="crew-agent-task w-full rounded-element border border-border-emphasized bg-background-default px-2 py-1.5 text-body text-text-default transition-[color,background-color,border-color,box-shadow] placeholder:text-text-muted hover:inset-ring-2 hover:inset-ring-border-emphasized/30"
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
            // The label sits above the value, as Task's and the picker's do, never centred
            // against a value that wraps (Q2-67).
            <div role="group" aria-labelledby={modelLabelId} className="flex flex-col gap-1.5">
              <span id={modelLabelId} className="text-label text-text-default">
                {agentCopy.model}
              </span>
              <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-label text-text-default">
                <span className="min-w-0 break-words">{shownName}</span>
                <ModelTierMarks provider={selectedProvider} privateOnly={privateOnly} />
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
              </span>
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
                privateOnly={privateOnly}
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
            summary={agentCopy.advancedSummary(here, alsoReads, folderText)}
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
              {folderText && <p className="text-supporting text-text-muted">{folderText}</p>}
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
        {/* Before Start, which it does NOT disable: the task names a file nobody shared here, so
            the agent would otherwise find something else by that name and say it used the file
            (Q2-15). The agent is told to say what it used instead. */}
        {unshared.length > 0 && (
          <Note tone="warning" icon={AlertTriangle} testId="crew-agent-file-warning">
            <p id={fileWarningId}>{agentCopy.fileNotShared(unshared, here)}</p>
          </Note>
        )}
        {/* Before Start, which it does NOT disable: the task names a file that two or more
            different shared files carry, and the agent will use the newest (Q3-02). */}
        {sameNamed.length > 0 && (
          <Note tone="info" icon={Info} testId="crew-agent-same-name-note">
            <div id={sameNameId}>
              {sameNamed.map((item) => (
                <p key={item.name.toLowerCase()}>
                  {agentCopy.sameNameShared(item.count, item.name, here, sharedWhen(item.sharedAt))}
                </p>
              ))}
            </div>
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
        {/* Start turns disabled while it works, which takes focus off it, so the same words are
            spoken here. */}
        <span
          className="sr-only"
          aria-live="polite"
          aria-atomic="true"
          data-crew-agent-start-status=""
        >
          {submitting ? agentCopy.starting : ''}
        </span>
        <div className="flex justify-end">
          <Button
            key="start"
            type="submit"
            disabled={startDisabled}
            aria-describedby={
              [
                mismatchText ? mismatchId : null,
                unshared.length > 0 ? fileWarningId : null,
                sameNamed.length > 0 ? sameNameId : null,
              ]
                .filter(Boolean)
                .join(' ') || undefined
            }
          >
            {submitting ? (
              <>
                <LoaderCircle className="crew-agent-start-spinner" aria-hidden="true" />
                {agentCopy.starting}
              </>
            ) : (
              agentCopy.start
            )}
          </Button>
        </div>
      </div>
    </form>
  );
}
