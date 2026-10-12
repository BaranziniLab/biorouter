import { useCallback, useEffect, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { Button } from '../ui/button';
import { errorMessage } from '../../utils/conversionUtils';
import { Terminal } from '../icons/app-icons';
import { CopyField } from '../ui/copy-field';
import InAppTerminalDock from '../InAppTerminalDock';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { InfoTip } from '../ui/info-tip';
import { agentCopy } from './copy';
import {
  AGENT_COMMAND_CONFIG,
  METERED_SIBLING,
  fetchCodingAgentStatus,
  type CodingAgentAvailability,
  type CodingAgentAuth,
  type CodingAgentKind,
  type CodingAgentStatusResponse,
} from './codingAgentStatus';
import './onboarding.css';
import { StatusPill, type PillTone } from './StatusPill';
import { useViewPrimaryVariant } from './viewPrimary';

/**
 * The last coding-agent status this renderer probed, shared by every surface.
 *
 * ⚠ Each probe spawns both vendor CLIs, so nothing may poll it. The catalog and
 * onboarding probe on mount and on an explicit re-check (`useCodingAgents`), and
 * record what they learned here; the model picker reads it through
 * {@link codingAgentStatusOnce}, which probes only if nothing has yet, so opening
 * Switch models never re-spawns what the catalog already asked (W2-PRV-5).
 */
let lastCodingAgentStatus: Promise<CodingAgentStatusResponse> | null = null;

/** The shared status, probing once if no surface has probed yet. A failed probe is not kept. */
export function codingAgentStatusOnce(): Promise<CodingAgentStatusResponse> {
  if (!lastCodingAgentStatus) {
    lastCodingAgentStatus = fetchCodingAgentStatus().catch((error: unknown) => {
      lastCodingAgentStatus = null;
      throw error;
    });
  }
  return lastCodingAgentStatus;
}

/** Test-only: forget the shared status. */
export function __resetCodingAgentStatusForTests(): void {
  lastCodingAgentStatus = null;
}

/**
 * Whether `providerId` names a coding agent (`claude_code`, `codex`): the keys of
 * {@link AGENT_COMMAND_CONFIG}, the one definition of the set.
 */
export function isCodingAgentProviderId(providerId: string): providerId is CodingAgentKind {
  return Object.prototype.hasOwnProperty.call(AGENT_COMMAND_CONFIG, providerId);
}

/**
 * Save the defaulted command key that marks a coding agent set up, the one
 * write "Use <agent>" makes. Shared with the model picker, which makes the same
 * write when a signed-in agent is chosen there (W2-PRV-5).
 */
export async function saveCodingAgentCommand(
  upsert: (key: string, value: unknown, isSecret: boolean) => Promise<void>,
  kind: CodingAgentKind
): Promise<void> {
  const { key, value } = AGENT_COMMAND_CONFIG[kind];
  await upsert(key, value, false);
}

/**
 * The coding-agent status probe, the per-state guidance, and the "use this
 * agent" write, hosted by the **provider catalog**'s accordion rows (Public's
 * "AI agents" section, on first run and in Settings) and by the configure
 * dialog's recovery panel.
 *
 * ⚠ One definition is the point. Every state machine, every sentence and every
 * `data-testid` below exists once, so a fix to the `signed_in_with_api_key` copy
 * cannot land on one screen and not the other. (A standalone onboarding card used
 * to wrap these too; nothing rendered it, and it was deleted with its chrome,
 * FR-S-12. Its tests moved to `codingAgentControls.test.tsx`.)
 */

/**
 * The height of the embedded terminal the user actually sees.
 *
 * ⚠ `InAppTerminalDock` sizes ITSELF for a chat pane —
 * `min(42vh, 380px, max(100px, calc(100% - 200px)))` — reserving 200px of its
 * parent for the transcript that normally sits above it. There is no transcript
 * here, so the dock is mounted in a box `CODING_AGENT_TERMINAL_RESERVE_PX` taller
 * than the visible region and the outer box clips the difference. Reusing the real
 * dock (rather than wiring a second xterm) is worth this one accommodation, but the
 * reserve below must stay in step with that `- 200px`.
 */
export const CODING_AGENT_TERMINAL_HEIGHT_PX = 240;
export const CODING_AGENT_TERMINAL_RESERVE_PX = 200;

/** What "Could not check" says when the failure carried no message of its own. */
export const CODING_AGENT_CHECK_FAILED = 'The check for installed coding agents returned an error.';

// The status pill has its own module so the local cards can use it without this one's imports.
export { StatusPill };
export type { PillTone };

/**
 * The pill for each auth state.
 *
 * `signed_in_with_api_key` is `info`, never `warning`: the CLI works, it would just
 * bill per token. Colouring it as a fault is exactly the mistake the state exists to
 * prevent.
 */
export function pillFor(auth: CodingAgentAuth): { tone: PillTone; label: string } {
  switch (auth.state) {
    case 'not_installed':
      return { tone: 'warning', label: 'Not installed' };
    case 'signed_out':
      return { tone: 'warning', label: 'Installed · not signed in' };
    case 'signed_in_with_api_key':
      return { tone: 'info', label: 'Signed in with an API key' };
    case 'signed_in_subscription':
      return { tone: 'success', label: 'Ready · signed in on your subscription' };
    case 'indeterminate':
      return { tone: 'warning', label: 'Status unclear' };
  }
}

/** "Check again" for every state except the one where the user just acted. */
export function recheckLabel(auth: CodingAgentAuth): string {
  return auth.state === 'signed_out' ? agentCopy.signedIn : agentCopy.checkAgain;
}

/**
 * A state's one instruction, with its reason one hover or Tab away (principle 2, vision FR-S-3).
 * Each state used to be a stack of explanatory paragraphs around the command.
 */
function Instruction({
  children,
  tipLabel,
  tip,
}: {
  children: React.ReactNode;
  tipLabel: string;
  tip: string;
}) {
  return (
    <p className="flex min-w-0 flex-wrap items-center text-supporting text-text-muted">
      <span className="min-w-0">{children}</span>
      <InfoTip label={tipLabel} help={tip} />
    </p>
  );
}

/** The actions under a state: one row on the 32px rung, never stretched to the panel's width. */
function Actions({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2 pt-1">{children}</div>;
}

/**
 * A command the user runs, never one Biorouter runs. Copyable, monospaced: the shared copy field
 * (a well with the Copy button inside it), so the command looks and copies like every other thing
 * Biorouter hands a person to paste. The label is the command itself, which is what the button's
 * name ("Copy npm install -g ...") has always said.
 */
export function CommandBlock({ command }: { command: string }) {
  return <CopyField value={command} label={command} />;
}

/** Everything both surfaces need, from one probe. */
export interface CodingAgentControls {
  agents: CodingAgentAvailability[] | null;
  isChecking: boolean;
  isRechecking: boolean;
  loadError: string | null;
  /** `false` on an explicit user re-check; `true` only for the mount probe. */
  refresh: (initial: boolean) => Promise<void>;
  connect: (agent: CodingAgentAvailability) => Promise<void>;
  connectingProviderId: string | null;
  terminalFor: CodingAgentKind | null;
  setTerminalFor: (kind: CodingAgentKind | null) => void;
}

/**
 * Probe both vendor CLIs once, and own the "use this agent" write.
 *
 * ⚠ **Fetched on mount and on an explicit user action only — never polled.** The
 * route spawns each vendor CLI (`--version` plus a credential probe), so a poll
 * would fork processes in the background for as long as the surface is on screen.
 * Both consumers mount this hook exactly once, which is what keeps that true:
 * `ProviderCatalog` mounts it at the tab panel, not per row.
 * A recovery dialog can supply the status just read by Save as `initialAgents`
 * to avoid spawning the same probes again on mount.
 *
 * `onRechecked` runs after an explicit re-check settles (never after the mount
 * probe). The catalog passes its provider-list refresh: the row's "Configured"
 * check comes from `GET /config/providers`, which requires the CLI to resolve,
 * so a re-check that changed "installed" has changed that answer too — and a
 * list read before it would show a check the pill beside it contradicts (F6).
 * The catalog also re-checks after any change to an agent's setup, for the
 * same reason in the other direction.
 */
export function useCodingAgents(
  onSuccess: (providerId: string) => void,
  onRechecked?: () => void,
  initialAgents?: CodingAgentAvailability[]
): CodingAgentControls {
  const { upsert } = useConfig();
  const [agents, setAgents] = useState<CodingAgentAvailability[] | null>(initialAgents ?? null);
  const [isChecking, setIsChecking] = useState(initialAgents === undefined);
  const needsInitialProbe = useRef(initialAgents === undefined);
  const [isRechecking, setIsRechecking] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [connectingProviderId, setConnectingProviderId] = useState<string | null>(null);
  const [terminalFor, setTerminalFor] = useState<CodingAgentKind | null>(null);

  // Ownership guard, as in LlamaServerInlineCard: `connect` awaits two config
  // writes and the surface can unmount between any two of them (onboarding
  // navigates away). A dead surface must not set state, toast, or drive the
  // navigation — but the writes it already made stand.
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // ⚠ Through a ref, so `refresh` keeps its empty dependency list. The mount
  // effect below re-runs whenever `refresh` changes identity, and a caller's
  // fresh closure per render would turn the one mount probe into a probe per
  // render — each of which spawns both vendor CLIs.
  const onRecheckedRef = useRef(onRechecked);
  useEffect(() => {
    onRecheckedRef.current = onRechecked;
  }, [onRechecked]);

  const refresh = useCallback(async (initial: boolean) => {
    if (initial) {
      setIsChecking(true);
    } else {
      setIsRechecking(true);
    }
    try {
      const status = await fetchCodingAgentStatus();
      lastCodingAgentStatus = Promise.resolve(status);
      if (!mountedRef.current) return;
      setAgents(status.agents);
      setLoadError(null);
    } catch (error) {
      console.error('Failed to check coding-agent CLIs:', error);
      if (!mountedRef.current) return;
      // The generated client throws the parsed response body for a failed status, which is an
      // object (`{ message }`), not an Error: `String()` of it printed "[object Object]" under
      // "Could not check". `errorMessage` reads the message of both shapes.
      setLoadError(errorMessage(error, CODING_AGENT_CHECK_FAILED));
    } finally {
      if (mountedRef.current) {
        setIsChecking(false);
        setIsRechecking(false);
        // Success or not: a probe that failed says nothing about the list, and
        // the change that prompted the re-check may still have moved it.
        if (!initial) onRecheckedRef.current?.();
      }
    }
  }, []);

  useEffect(() => {
    if (needsInitialProbe.current) void refresh(true);
  }, [refresh]);

  const connect = useCallback(
    async (agent: CodingAgentAvailability) => {
      if (connectingProviderId) return;
      setConnectingProviderId(agent.providerId);
      const { key } = AGENT_COMMAND_CONFIG[agent.kind];
      try {
        // Explicitly saving the (defaulted) command key is what marks the provider
        // configured — `check_provider_configured` reports a required key as
        // missing until it is actually written, default or no default. Without this
        // line the surface would say "Ready" and the provider would still be listed
        // as needing setup. Re-check ownership after EVERY await so a dead surface
        // stops writing config mid-sequence.
        //
        // ⚠ And nothing else. This used to write `BIOROUTER_PROVIDER` too, before
        // any model was chosen: the model every new chat starts on, in every
        // window, moved on this click, and a Cancel in the model dialog that
        // follows left it paired with the previous provider's model (W2-PRV-5).
        // The model dialog `onSuccess` opens writes the provider and the model
        // together, to the scope the person picks there.
        await saveCodingAgentCommand(upsert, agent.kind);
        if (!mountedRef.current) return;
        toastService.success({
          title: `${agent.displayName} ready`,
          msg: `Biorouter will drive your installed ${agent.displayName} CLI on your own subscription.`,
        });
        onSuccess(agent.providerId);
      } catch (error) {
        toastService.error({
          title: 'Could not select this provider',
          msg: `Failed to save ${key}: ${errorMessage(error)}`,
          traceback: error instanceof Error ? error.stack || '' : '',
        });
      } finally {
        // ⚠ `finally`, so SUCCESS clears it too.
        //
        // This used to clear only in the catch arm, on the reasoning that a
        // successful connect hands off and never comes back. It does come back:
        // `ProviderGuard` opens the Choose Model modal BESIDE the still-mounted
        // onboarding tree rather than unmounting it, and closing that modal
        // without picking a model returns the user to a row whose button still
        // reads "Connecting…" and is disabled - along with the other agent's,
        // since both are `disabled={connectingProviderId !== null}` and
        // `connect` early-returns while it is set. Nothing here resets it, so
        // neither coding agent could be chosen again for the rest of the run,
        // and the config write has already happened so a relaunch skips
        // onboarding entirely.
        if (mountedRef.current) setConnectingProviderId(null);
      }
    },
    [connectingProviderId, onSuccess, upsert]
  );

  return {
    agents,
    isChecking,
    isRechecking,
    loadError,
    refresh,
    connect,
    connectingProviderId,
    terminalFor,
    setTerminalFor,
  };
}

/**
 * An in-app shell, offered *beside* the copyable command and never instead of it.
 * A user who prefers their own terminal loses nothing; a user who does not has to
 * leave the app to type one line.
 */
function TerminalOption({
  agent,
  controls,
}: {
  agent: CodingAgentAvailability;
  controls: CodingAgentControls;
}) {
  const open = controls.terminalFor === agent.kind;
  return (
    <Button
      type="button"
      variant="ghost"
      onClick={() => controls.setTerminalFor(open ? null : agent.kind)}
      data-testid={`coding-agent-terminal-toggle-${agent.providerId}`}
    >
      <Terminal />
      {open ? agentCopy.hideTerminal : agentCopy.openTerminal}
    </Button>
  );
}

function EmbeddedTerminal({
  agent,
  controls,
}: {
  agent: CodingAgentAvailability;
  controls: CodingAgentControls;
}) {
  if (controls.terminalFor !== agent.kind) return null;
  return (
    <div
      className="br-first-run-box overflow-hidden"
      style={{ height: CODING_AGENT_TERMINAL_HEIGHT_PX }}
      data-testid={`coding-agent-terminal-${agent.providerId}`}
    >
      {/* See CODING_AGENT_TERMINAL_RESERVE_PX: the dock reserves 200px of its
          parent for a chat transcript that does not exist here, so the mount box
          is that much taller and the parent clips the remainder. */}
      <div style={{ height: CODING_AGENT_TERMINAL_HEIGHT_PX + CODING_AGENT_TERMINAL_RESERVE_PX }}>
        <InAppTerminalDock
          open
          onClose={() => controls.setTerminalFor(null)}
          onEmptied={() => controls.setTerminalFor(null)}
        />
      </div>
    </div>
  );
}

function RecheckButton({
  agent,
  controls,
}: {
  agent: CodingAgentAvailability;
  controls: CodingAgentControls;
}) {
  return (
    <Button
      type="button"
      variant="secondary"
      onClick={() => void controls.refresh(false)}
      disabled={controls.isRechecking}
      data-testid={`coding-agent-recheck-${agent.providerId}`}
    >
      {controls.isRechecking ? agentCopy.checking : recheckLabel(agent.auth)}
    </Button>
  );
}

/**
 * The per-state guidance for one agent — the body of the row, without its
 * heading or pill.
 *
 * Five arms, one per `AuthState`, and each one's reasoning is recorded on the
 * arm rather than in a summary that can drift from it.
 */
export function CodingAgentBody({
  agent,
  controls,
  onRetryConfiguration,
  isRetrying = false,
}: {
  agent: CodingAgentAvailability;
  controls: CodingAgentControls;
  onRetryConfiguration?: () => void;
  isRetrying?: boolean;
}) {
  // "Use <agent>" is coral unless another open panel in the tab holds the view's primary (FR-R5-2).
  const primaryVariant = useViewPrimaryVariant();
  switch (agent.auth.state) {
    // 1. Nothing installed. The install command is SHOWN, never run: installing
    //    another vendor's toolchain is the user's decision, and Anthropic's
    //    installer is a piped shell script. There is deliberately no
    //    "Install for me" button anywhere here.
    case 'not_installed':
      return (
        <div className="space-y-2">
          <Instruction
            tipLabel={agentCopy.notInstalledTipLabel}
            tip={agentCopy.notInstalledTip(AGENT_COMMAND_CONFIG[agent.kind].key)}
          >
            {agentCopy.notInstalled}
          </Instruction>
          <CommandBlock command={agent.installHint} />
          <Actions>
            <RecheckButton agent={agent} controls={controls} />
            {/* A shell the user types the install line into is still the user
                installing it. What is prohibited is Biorouter running it. */}
            <TerminalOption agent={agent} controls={controls} />
          </Actions>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 2. Installed, no credential. The sign-in happens in the user's own
    //    terminal, against the vendor — Biorouter never brokers it.
    case 'signed_out':
      return (
        <div className="space-y-2">
          <Instruction tipLabel={agentCopy.signedOutTipLabel} tip={agentCopy.signedOutTip}>
            {agentCopy.signedOut}
          </Instruction>
          <CommandBlock command={agent.loginCommand} />
          <Actions>
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </Actions>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 3. Signed in — but with an API key. NOT a failure, and it must not look like
    //    one: the CLI runs fine, it would just bill per token, and this provider
    //    strips API credentials from the environment it starts, so it cannot run.
    //    One instruction names both ways out; the reason is its tip.
    case 'signed_in_with_api_key':
      return (
        <div className="space-y-2">
          <Instruction tipLabel={agentCopy.apiKeyTipLabel} tip={agentCopy.apiKeyTip}>
            {agentCopy.apiKeyLead}{' '}
            <span className="text-text-default">{METERED_SIBLING[agent.kind]}</span>{' '}
            {agentCopy.apiKeyTail}
          </Instruction>
          <CommandBlock command={agent.loginCommand} />
          <Actions>
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </Actions>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 4. Ready. Selecting persists the defaulted command key (which is what makes
    //    the provider report as configured) and hands off to model selection. The
    //    plan and account are on the version line above; there is nothing to explain.
    case 'signed_in_subscription':
      return (
        <Actions>
          <Button
            type="button"
            variant={primaryVariant}
            onClick={() =>
              onRetryConfiguration ? onRetryConfiguration() : void controls.connect(agent)
            }
            disabled={isRetrying || controls.connectingProviderId !== null || controls.isRechecking}
            data-testid={`coding-agent-connect-${agent.providerId}`}
          >
            {isRetrying
              ? agentCopy.checkingConfiguration
              : onRetryConfiguration
                ? agentCopy.retryConfiguration
                : controls.connectingProviderId === agent.providerId
                  ? agentCopy.connecting
                  : agentCopy.use(agent.displayName)}
          </Button>
          <RecheckButton agent={agent} controls={controls} />
        </Actions>
      );

    // 5. The probe ran but could not be read. Telling a user who IS signed in to
    //    sign in is worse than admitting we do not know, so this arm offers no
    //    login command — only the reason and a re-check. The sentence and the
    //    daemon's detail are one style: they used to be two inks and two sizes.
    case 'indeterminate':
      return (
        <div className="space-y-2">
          <p className="text-supporting text-text-muted">{agentCopy.indeterminate}</p>
          <p
            className="break-words text-supporting text-text-muted"
            data-testid={`coding-agent-detail-${agent.providerId}`}
          >
            {agent.auth.detail}
          </p>
          <Actions>
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </Actions>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );
  }
}

/**
 * The version number in a CLI's raw `--version` line: `2.1.283` out of "2.1.283 (Claude Code)",
 * `0.157.1` out of "codex-cli 0.157.1". The two vendors print two shapes, and the row's own name
 * already says which CLI it is, so the panel shows the number alone (vision FR-R3-6, principle
 * 10). A line with no number is shown as it came.
 */
export function agentVersionNumber(raw: string): string {
  return raw.match(/\d+(?:\.\d+)+(?:-[0-9A-Za-z.]+)?/)?.[0] ?? raw.trim();
}

/** "max" -> "Max plan", "team_premium" -> "Team premium plan". */
export function agentPlanName(plan: string): string {
  const words = plan.replace(/[_-]+/g, ' ').trim();
  return agentCopy.plan(`${words.charAt(0).toUpperCase()}${words.slice(1)}`);
}

/**
 * The version line, when the probe resolved a version or a path. The version is what a person
 * reads; where the CLI lives is a path, so it sits behind the line's tooltip (principle 10, V4-26)
 * instead of on the row in mono. The line takes focus, like a provider's description, so the
 * tooltip is reachable by keyboard. With a path and no version the line says only that the CLI was
 * found.
 *
 * Both agents' lines have one shape (vision FR-R3-6): "Version 2.1.283 · Max plan · ada@…" and
 * "Version 0.157.1", the number in mono at the line's 12px (a version is a machine string,
 * principle 3) and everything else in the line's sans face.
 */
export function CodingAgentProvenance({ agent }: { agent: CodingAgentAvailability }) {
  // The plan and the account join the version on one line (FR-R1-8): a ready agent's panel was a
  // version line, a plan line and a paragraph before its button.
  const plan =
    agent.auth.state === 'signed_in_subscription'
      ? [agent.auth.plan ? agentPlanName(agent.auth.plan) : null, agent.auth.account ?? null]
          .filter(Boolean)
          .join(' · ')
      : '';
  const hasVersion = Boolean(agent.version || agent.path);
  if (!hasVersion && !plan) return null;
  const version = hasVersion ? (
    <span
      tabIndex={agent.path ? 0 : undefined}
      data-testid={`coding-agent-version-${agent.providerId}`}
      className="cursor-default rounded-element biorouter-focus-surface"
    >
      {agent.version ? (
        <>
          {agentCopy.version} <span className="font-mono">{agentVersionNumber(agent.version)}</span>
        </>
      ) : (
        agentCopy.installed
      )}
    </span>
  ) : null;
  return (
    <p className="min-w-0 break-words text-supporting text-text-muted">
      {version && agent.path ? (
        <Tooltip>
          <TooltipTrigger asChild>{version}</TooltipTrigger>
          <TooltipContent side="bottom" className="max-w-96 break-all text-wrap font-mono">
            {agent.path}
          </TooltipContent>
        </Tooltip>
      ) : (
        version
      )}
      {version && plan ? ' · ' : null}
      {plan ? <span>{plan}</span> : null}
    </p>
  );
}

/** "Could not check" — the probe itself failed, so there are no rows to draw. */
export function CodingAgentLoadError({
  message,
  controls,
}: {
  message: string;
  controls: CodingAgentControls;
}) {
  return (
    <div className="space-y-2">
      <StatusPill tone="warning">{agentCopy.couldNotCheck}</StatusPill>
      <p
        className="break-words text-supporting text-text-muted"
        data-testid="coding-agent-load-error"
      >
        {message}
      </p>
      <Actions>
        <Button
          type="button"
          variant="secondary"
          onClick={() => void controls.refresh(false)}
          disabled={controls.isRechecking}
          data-testid="coding-agent-retry"
        >
          {controls.isRechecking ? agentCopy.checking : agentCopy.tryAgain}
        </Button>
      </Actions>
    </div>
  );
}
