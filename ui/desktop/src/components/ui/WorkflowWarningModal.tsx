import * as React from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from './button';
import { Note } from './note';
import { InfoTip } from './info-tip';
import MarkdownContent from '../MarkdownContent';
import {
  decisionModelLabel,
  fetchDecisionModels,
  formatProbability,
  parseDecisionSettings,
  type DecisionModelEntry,
  type DecisionModelRef,
  type MonitorAction,
} from '../../utils/sessionDecisions';
import { parseGateModel } from '../../utils/phaseGates';

/**
 * What a workflow declares to supervise the chat it starts: its `decision_settings`
 * (the default decision model and the monitors) and its `orchestration_plan` (the
 * phases and their gates), as the workflow file carries them. Both are read
 * tolerantly: a block this build cannot read is left out of the summary, never
 * shown as something it is not.
 */
export interface WorkflowSupervision {
  decisionSettings?: unknown;
  orchestrationPlan?: unknown;
}

export const WORKFLOW_SUPERVISION_COPY = {
  title: 'Supervision',
  defaultModel: 'Default decision model',
  configuredDefault: 'Biorouter’s configured decision model',
  monitors: 'Monitors',
  chatDefault: 'chat default',
  actions: {
    stop: 'stops the turn',
    notify: 'notifies you',
    steer: 'tells the agent',
  } as Record<MonitorAction, string>,
  off: 'off',
  gates: 'Phase gates',
  plan: (title: string, phases: number) =>
    `Phase plan “${title}”, ${phases} ${phases === 1 ? 'phase' : 'phases'}.`,
  gate: (questions: number, threshold: string, attempts: number) =>
    `${questions} ${questions === 1 ? 'question' : 'questions'} at P(yes) ≥ ${threshold}, ${attempts} ${attempts === 1 ? 'attempt' : 'attempts'}`,
  gatesBind:
    'The agent cannot change, skip or remove these gates; only you can pass, skip or retry one.',
  reads: 'A monitor’s decision model reads the newest messages of this chat.',
  private: 'Private',
  public: 'Public',
};

const supervisionCopy = WORKFLOW_SUPERVISION_COPY;

interface SummaryMonitor {
  id: string;
  name: string;
  model: DecisionModelRef | null;
  action: MonitorAction;
  enabled: boolean;
}

interface SummaryGate {
  phaseId: string;
  title: string;
  questions: number;
  threshold: number;
  attempts: number;
  model: DecisionModelRef | null;
}

interface SupervisionView {
  defaultModel: DecisionModelRef | null;
  monitors: SummaryMonitor[];
  plan: { title: string; phases: number } | null;
  gates: SummaryGate[];
}

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/** The plan block as the workflow file writes it (`WorkflowPlan`, snake_case keys). */
function readPlan(value: unknown): Pick<SupervisionView, 'plan' | 'gates'> {
  const plan = record(value);
  const phases = Array.isArray(plan?.phases) ? plan.phases : [];
  if (!plan || phases.length === 0) return { plan: null, gates: [] };
  const gates: SummaryGate[] = [];
  for (const entry of phases) {
    const phase = record(entry);
    const gate = record(phase?.gate);
    const id = text(phase?.id);
    if (!phase || !gate || !id) continue;
    const threshold = typeof gate.threshold === 'number' ? gate.threshold : 0.8;
    const attempts = typeof gate.max_attempts === 'number' ? gate.max_attempts : 3;
    gates.push({
      phaseId: id,
      title: text(phase.title) ?? id,
      questions: Object.keys(record(gate.questions) ?? {}).length,
      threshold,
      attempts,
      model: parseGateModel(text(gate.model)),
    });
  }
  return { plan: { title: text(plan.title) ?? '', phases: phases.length }, gates };
}

/** What the summary draws, or `null` when the workflow declares no supervision. */
export function workflowSupervisionView(
  supervision: WorkflowSupervision | null | undefined
): SupervisionView | null {
  if (!supervision) return null;
  const settings = parseDecisionSettings(supervision.decisionSettings);
  const { plan, gates } = readPlan(supervision.orchestrationPlan);
  const monitors = (settings?.monitors ?? []).map((monitor) => ({
    id: monitor.id,
    name: monitor.name,
    model: monitor.model,
    action: monitor.action,
    enabled: monitor.enabled,
  }));
  if (!settings?.defaultModel && monitors.length === 0 && !plan) return null;
  return { defaultModel: settings?.defaultModel ?? null, monitors, plan, gates };
}

function namedModels(view: SupervisionView): DecisionModelRef[] {
  return [
    view.defaultModel,
    ...view.monitors.map((monitor) => monitor.model),
    ...view.gates.map((gate) => gate.model),
  ].filter((model): model is DecisionModelRef => model !== null);
}

/** `joey-1 · Public`: the model's name, and its tier when the decision model list says it. */
function modelWords(model: DecisionModelRef, models: readonly DecisionModelEntry[]): string {
  const entry = models.find(
    (candidate) => candidate.provider === model.provider && candidate.model === model.model
  );
  const tier =
    entry?.tier === 'private'
      ? supervisionCopy.private
      : entry?.tier === 'public'
        ? supervisionCopy.public
        : null;
  return [entry?.displayName ?? decisionModelLabel(model) ?? model.model, tier]
    .filter(Boolean)
    .join(' · ');
}

/**
 * A workflow's supervision in a few lines: the default decision model, each
 * monitor (its name, its model or "chat default", that model's tier, what it
 * does), and each gated phase. Read-only, and the same wherever a workflow is
 * shown before it runs or while it is edited, so the two never describe one
 * file differently.
 *
 * The tiers come from `GET /decisions/models` (`models`, or read here when not
 * given); a model the list does not name, or a list that could not be read,
 * shows its name alone.
 */
export function WorkflowSupervisionSummary({
  supervision,
  models: givenModels,
  children,
}: {
  supervision: WorkflowSupervision | null | undefined;
  models?: readonly DecisionModelEntry[];
  /** A closing line of the caller's own. */
  children?: React.ReactNode;
}) {
  const view = React.useMemo(() => workflowSupervisionView(supervision), [supervision]);
  const [fetched, setFetched] = React.useState<DecisionModelEntry[]>([]);
  const wantsList = !!view && !givenModels && namedModels(view).length > 0;
  React.useEffect(() => {
    if (!wantsList) return;
    const controller = new AbortController();
    fetchDecisionModels(controller.signal)
      .then((entries) => {
        if (!controller.signal.aborted) setFetched(entries);
      })
      // No list: each model is named without its tier.
      .catch(() => {});
    return () => controller.abort();
  }, [wantsList]);
  if (!view) return null;
  const models = givenModels ?? fetched;
  return (
    <section
      aria-label={supervisionCopy.title}
      className="biorouter-modal-panel p-4 rounded-container"
      data-testid="workflow-supervision"
    >
      <h3 className="mb-3 text-label text-text-default">{supervisionCopy.title}</h3>
      <div className="space-y-4">
        <p className="text-body text-text-default" data-testid="workflow-supervision-default">
          {supervisionCopy.defaultModel}:{' '}
          {view.defaultModel
            ? modelWords(view.defaultModel, models)
            : supervisionCopy.configuredDefault}
        </p>
        {view.monitors.length > 0 ? (
          <div>
            <h4 className="mb-1 text-label text-text-default">{supervisionCopy.monitors}</h4>
            <ul className="flex flex-col gap-1">
              {view.monitors.map((monitor) => (
                <li
                  key={monitor.id}
                  className="text-body text-text-default"
                  data-testid={`workflow-supervision-monitor-${monitor.id}`}
                >
                  {[
                    monitor.name,
                    monitor.model ? modelWords(monitor.model, models) : supervisionCopy.chatDefault,
                    supervisionCopy.actions[monitor.action],
                    monitor.enabled ? null : supervisionCopy.off,
                  ]
                    .filter(Boolean)
                    .join(' · ')}
                </li>
              ))}
            </ul>
          </div>
        ) : null}
        {view.plan ? (
          <div>
            <h4 className="mb-1 text-label text-text-default">{supervisionCopy.gates}</h4>
            <p className="text-body text-text-default">
              {supervisionCopy.plan(view.plan.title, view.plan.phases)}
            </p>
            {view.gates.length > 0 ? (
              <>
                <ul className="flex flex-col gap-1">
                  {view.gates.map((gate) => (
                    <li
                      key={gate.phaseId}
                      className="text-body text-text-default"
                      data-testid={`workflow-supervision-gate-${gate.phaseId}`}
                    >
                      {[
                        gate.title,
                        supervisionCopy.gate(
                          gate.questions,
                          formatProbability(gate.threshold),
                          gate.attempts
                        ),
                        gate.model ? modelWords(gate.model, models) : supervisionCopy.chatDefault,
                      ].join(' · ')}
                    </li>
                  ))}
                </ul>
                <p className="text-supporting text-text-muted">{supervisionCopy.gatesBind}</p>
              </>
            ) : null}
          </div>
        ) : null}
        {view.monitors.length > 0 ? (
          <p className="text-supporting text-text-muted">{supervisionCopy.reads}</p>
        ) : null}
        {children}
      </div>
    </section>
  );
}

/** The dialog's words, so a test reads what the dialog says. */
export const WORKFLOW_WARNING_COPY = {
  notRunBefore: "You haven't run this workflow before.",
  trustSource: 'Only run it if you trust its source.',
  hiddenCharacters: 'Hidden characters in this workflow will be ignored.',
  hiddenCharactersTipLabel: 'hidden characters',
  hiddenCharactersTip:
    'They could be used for malicious purposes, so Biorouter ignores them for your safety.',
} as const;

interface WorkflowWarningModalProps {
  isOpen: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  workflowDetails: {
    title?: string;
    description?: string;
    instructions?: string;
    /** The monitors and phase plan the workflow starts its chat with, when it declares any. */
    supervision?: WorkflowSupervision;
  };
  hasSecurityWarnings?: boolean;
  /** For tests: the decision models, instead of `GET /decisions/models`. */
  decisionModels?: DecisionModelEntry[];
}

/**
 * Asked before a workflow the person has not run before (or one carrying hidden characters) runs.
 *
 * A `ModalShell` at size `lg` with a scrolling body (spec 2.6; vision round 7, V3-R7-1 named it
 * among the raw `DialogContent` call sites): pinned at the top edge, the title in the header above
 * its rule, 16px insets, and the preview scrolling between the header and the footer rules. It
 * was a raw `DialogContent` 80% of the window wide, centred, with 24px insets. It must be answered
 * (`purpose="required"`): Escape and an outside press do nothing and there is no ×, as before.
 * The warning sentence is the dialog's description (`describedBy`), in body text.
 *
 * A workflow that declares supervision (monitors, a phase plan with gates) shows it under the
 * preview, so the person knows what will read the chat and what will hold the agent before
 * trusting it.
 */
export function WorkflowWarningModal({
  isOpen,
  onConfirm,
  onCancel,
  workflowDetails,
  hasSecurityWarnings = false,
  decisionModels,
}: WorkflowWarningModalProps) {
  const descriptionId = React.useId();
  return (
    <ModalShell
      open={isOpen}
      // Required: nothing dismisses it. If anything ever did, cancelling is the safe answer.
      onOpenChange={(open) => !open && onCancel()}
      size="lg"
      purpose="required"
      scrollBody
      title={hasSecurityWarnings ? 'Security warning' : 'New workflow warning'}
      describedBy={descriptionId}
      bodyClassName="flex flex-col gap-3 py-3"
      footer={
        <>
          <Button variant="secondary" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onConfirm}>Trust and execute</Button>
        </>
      }
    >
      {/* Two short sentences: why it asks, and the decision (owner, 2026-11-13). The title
          already says the workflow is new. */}
      <p id={descriptionId} className="text-body text-text-muted">
        {!hasSecurityWarnings && `${WORKFLOW_WARNING_COPY.notRunBefore} `}
        {WORKFLOW_WARNING_COPY.trustSource}
      </p>

      {/* The warning stays visible; why hidden characters are dropped is its InfoTip. */}
      {hasSecurityWarnings && (
        <Note tone="warning">
          <span className="inline-flex flex-wrap items-center">
            <span>{WORKFLOW_WARNING_COPY.hiddenCharacters}</span>
            <InfoTip
              label={WORKFLOW_WARNING_COPY.hiddenCharactersTipLabel}
              help={WORKFLOW_WARNING_COPY.hiddenCharactersTip}
            />
          </span>
        </Note>
      )}

      <div className="biorouter-modal-panel p-4 rounded-container">
        <h3 className="mb-3 text-label text-text-default">Workflow preview</h3>
        <div className="space-y-4">
          {workflowDetails.title && (
            <p className="text-body text-text-default">
              <strong className="font-semibold">Title</strong> {workflowDetails.title}
            </p>
          )}
          {workflowDetails.description && (
            <p className="text-body text-text-default">
              <strong className="font-semibold">Description</strong> {workflowDetails.description}
            </p>
          )}
          {workflowDetails.instructions && (
            <div>
              <h4 className="mb-1 text-label text-text-default">Instructions</h4>
              <MarkdownContent content={workflowDetails.instructions} className="text-body" />
            </div>
          )}
        </div>
      </div>

      {isOpen ? (
        <WorkflowSupervisionSummary
          supervision={workflowDetails.supervision}
          models={decisionModels}
        />
      ) : null}
    </ModalShell>
  );
}
