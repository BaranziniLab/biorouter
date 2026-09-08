import { useCallback, useEffect, useRef, useState } from 'react';
import { useConfig } from '../ConfigContext';
import { toastService } from '../../toasts';
import { Button } from '../ui/button';
import { Check, Copy, Terminal } from '../icons/app-icons';
import InAppTerminalDock from '../InAppTerminalDock';
import {
  AGENT_COMMAND_CONFIG,
  METERED_SIBLING,
  fetchCodingAgentStatus,
  type CodingAgentAvailability,
  type CodingAgentAuth,
  type CodingAgentKind,
} from './codingAgentStatus';

/**
 * The coding-agent status probe, the per-state guidance, and the "use this
 * agent" write — extracted from `CodingAgentInlineCard` so the **provider
 * catalog** hosts the identical UI rather than a second copy of it.
 *
 * ⚠ The extraction is the point. The two surfaces differ only in their *shell*
 * (the onboarding card's `<section>` and heading vs. the catalog's accordion
 * row); every state machine, every sentence and every `data-testid` below has
 * exactly one definition, so a fix to the `signed_in_with_api_key` copy cannot
 * land on one screen and not the other.
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

type PillTone = 'success' | 'warning' | 'info';

const PILL_DOT: Record<PillTone, string> = {
  success: 'bg-background-success',
  warning: 'bg-background-warning',
  info: 'bg-background-info',
};

export function StatusPill({ tone, children }: { tone: PillTone; children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1.5 text-[11px] text-text-muted">
      <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${PILL_DOT[tone]}`} />
      {children}
    </span>
  );
}

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
  return auth.state === 'signed_out' ? "I've signed in" : 'Check again';
}

/** A command the user runs, never one Biorouter runs. Copyable, monospaced. */
export function CommandBlock({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    []
  );

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      console.error('Failed to copy command to clipboard:', error);
    }
  };

  return (
    <div className="flex min-w-0 items-center gap-2 rounded-md border border-border-subtle bg-background-muted p-2">
      <code className="min-w-0 flex-1 break-all font-mono text-[11px] leading-relaxed text-text-default">
        {command}
      </code>
      <Button
        type="button"
        variant="ghost"
        size="xs"
        shape="pill"
        onClick={copy}
        className="flex-shrink-0"
        aria-label={copied ? `Copied ${command}` : `Copy ${command}`}
      >
        {copied ? <Check /> : <Copy />}
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  );
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
 */
export function useCodingAgents(onSuccess: (providerId: string) => void): CodingAgentControls {
  const { upsert } = useConfig();
  const [agents, setAgents] = useState<CodingAgentAvailability[] | null>(null);
  const [isChecking, setIsChecking] = useState(true);
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

  const refresh = useCallback(async (initial: boolean) => {
    if (initial) {
      setIsChecking(true);
    } else {
      setIsRechecking(true);
    }
    try {
      const status = await fetchCodingAgentStatus();
      if (!mountedRef.current) return;
      setAgents(status.agents);
      setLoadError(null);
    } catch (error) {
      console.error('Failed to check coding-agent CLIs:', error);
      if (!mountedRef.current) return;
      setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      if (mountedRef.current) {
        setIsChecking(false);
        setIsRechecking(false);
      }
    }
  }, []);

  useEffect(() => {
    void refresh(true);
  }, [refresh]);

  const connect = useCallback(
    async (agent: CodingAgentAvailability) => {
      if (connectingProviderId) return;
      setConnectingProviderId(agent.providerId);
      const { key, value } = AGENT_COMMAND_CONFIG[agent.kind];
      try {
        // Explicitly saving the (defaulted) command key is what marks the provider
        // configured — `check_provider_configured` reports a required key as
        // missing until it is actually written, default or no default. Without this
        // line the surface would say "Ready" and the provider would still be listed
        // as needing setup. Re-check ownership after EVERY await so a dead surface
        // stops writing config mid-sequence.
        await upsert(key, value, false);
        if (!mountedRef.current) return;
        await upsert('BIOROUTER_PROVIDER', agent.providerId, false);
        if (!mountedRef.current) return;
        toastService.success({
          title: `${agent.displayName} ready`,
          msg: `Biorouter will drive your installed ${agent.displayName} CLI on your own subscription.`,
        });
        onSuccess(agent.providerId);
      } catch (error) {
        toastService.error({
          title: 'Could not select this provider',
          msg: `Failed to save ${key}: ${error instanceof Error ? error.message : String(error)}`,
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
      size="sm"
      shape="pill"
      onClick={() => controls.setTerminalFor(open ? null : agent.kind)}
      data-testid={`coding-agent-terminal-toggle-${agent.providerId}`}
    >
      <Terminal />
      {open ? 'Hide terminal' : 'Open a terminal here'}
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
      className="overflow-hidden rounded-md border border-border-subtle bg-background-muted"
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
      variant="outline"
      size="lg"
      shape="pill"
      onClick={() => void controls.refresh(false)}
      disabled={controls.isRechecking}
      className="w-full px-4 sm:w-auto"
      data-testid={`coding-agent-recheck-${agent.providerId}`}
    >
      {controls.isRechecking ? 'Checking…' : recheckLabel(agent.auth)}
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
}: {
  agent: CodingAgentAvailability;
  controls: CodingAgentControls;
}) {
  switch (agent.auth.state) {
    // 1. Nothing installed. The install command is SHOWN, never run: installing
    //    another vendor's toolchain is the user's decision, and Anthropic's
    //    installer is a piped shell script. There is deliberately no
    //    "Install for me" button anywhere here.
    case 'not_installed':
      return (
        <div className="space-y-2">
          <p className="text-xs text-text-muted leading-relaxed">
            Install it yourself, then check again. Biorouter shows the command rather than running
            it — this installs another vendor&apos;s toolchain on your machine, so it stays your
            call.
          </p>
          <CommandBlock command={agent.installHint} />
          <p className="text-[11px] text-text-muted">
            Already installed somewhere unusual (nvm, volta, bun, asdf)? Set{' '}
            <code className="font-mono">{AGENT_COMMAND_CONFIG[agent.kind].key}</code> to its full
            path in Settings instead.
          </p>
          <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
            <RecheckButton agent={agent} controls={controls} />
            {/* A shell the user types the install line into is still the user
                installing it. What is prohibited is Biorouter running it. */}
            <TerminalOption agent={agent} controls={controls} />
          </div>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 2. Installed, no credential. The sign-in happens in the user's own
    //    terminal, against the vendor — Biorouter never brokers it.
    case 'signed_out':
      return (
        <div className="space-y-2">
          <p className="text-xs text-text-muted leading-relaxed">
            Run this yourself, in a terminal:
          </p>
          <CommandBlock command={agent.loginCommand} />
          <p className="text-[11px] text-text-muted leading-relaxed">
            Biorouter deliberately does not perform the sign-in: the credential stays between you
            and the vendor, and never passes through Biorouter.
          </p>
          <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </div>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 3. Signed in — but with an API key. NOT a failure, and it must not look like
    //    one: the CLI runs fine, it would just bill per token, and this provider
    //    strips API credentials from the environment it starts, so it cannot run.
    case 'signed_in_with_api_key':
      return (
        <div className="space-y-2">
          <p className="text-xs text-text-muted leading-relaxed">
            The CLI works — it is just signed in with an API key, so a run would bill per token.
            This provider exists specifically to use your own plan, and it removes API credentials
            from the environment it starts, so it cannot run this way.
          </p>
          <p className="text-xs text-text-muted leading-relaxed">
            Either sign in with your subscription:
          </p>
          <CommandBlock command={agent.loginCommand} />
          <p className="text-[11px] text-text-muted leading-relaxed">
            …or keep the API key and use the metered{' '}
            <span className="text-text-default">{METERED_SIBLING[agent.kind]}</span> provider
            instead, under API providers.
          </p>
          <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </div>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );

    // 4. Ready. Selecting persists the defaulted command key (which is what makes
    //    the provider report as configured) and hands off to model selection.
    case 'signed_in_subscription': {
      const { plan, account } = agent.auth;
      return (
        <div className="space-y-2">
          {(plan || account) && (
            <p className="text-[11px] text-text-muted break-words">
              {[plan ? `Plan: ${plan}` : null, account ? `Account: ${account}` : null]
                .filter(Boolean)
                .join(' · ')}
            </p>
          )}
          <p className="text-xs text-text-muted leading-relaxed">
            Turns run through your own subscription, so there is nothing metered to configure here.
          </p>
          <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
            <Button
              type="button"
              size="lg"
              shape="pill"
              onClick={() => void controls.connect(agent)}
              disabled={controls.connectingProviderId !== null}
              className="w-full px-4 sm:w-auto"
              data-testid={`coding-agent-connect-${agent.providerId}`}
            >
              {controls.connectingProviderId === agent.providerId
                ? 'Connecting…'
                : `Use ${agent.displayName}`}
            </Button>
            <RecheckButton agent={agent} controls={controls} />
          </div>
        </div>
      );
    }

    // 5. The probe ran but could not be read. Telling a user who IS signed in to
    //    sign in is worse than admitting we do not know, so this arm offers no
    //    login command — only the reason and a re-check.
    case 'indeterminate':
      return (
        <div className="space-y-2">
          <p className="text-xs text-text-muted leading-relaxed">
            Biorouter could not tell whether this CLI is usable, so it is not guessing at the fix.
          </p>
          <p
            className="break-words rounded-md border border-border-subtle bg-background-muted p-2 font-mono text-[11px] text-text-default"
            data-testid={`coding-agent-detail-${agent.providerId}`}
          >
            {agent.auth.detail}
          </p>
          <div className="flex flex-col items-stretch gap-2 pt-1 sm:flex-row sm:flex-wrap sm:items-center sm:gap-x-3">
            <RecheckButton agent={agent} controls={controls} />
            <TerminalOption agent={agent} controls={controls} />
          </div>
          <EmbeddedTerminal agent={agent} controls={controls} />
        </div>
      );
  }
}

/** The version/path line, when the probe resolved either. */
export function CodingAgentProvenance({ agent }: { agent: CodingAgentAvailability }) {
  if (!agent.version && !agent.path) return null;
  return (
    <p className="min-w-0 break-all font-mono text-[11px] text-text-muted">
      {[agent.version, agent.path].filter(Boolean).join(' · ')}
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
    <div className="space-y-3">
      <StatusPill tone="warning">Could not check</StatusPill>
      <p
        className="break-words text-xs text-text-muted leading-relaxed"
        data-testid="coding-agent-load-error"
      >
        {message}
      </p>
      <Button
        type="button"
        variant="outline"
        size="lg"
        shape="pill"
        onClick={() => void controls.refresh(false)}
        disabled={controls.isRechecking}
        className="w-full px-4 sm:w-auto"
        data-testid="coding-agent-retry"
      >
        {controls.isRechecking ? 'Checking…' : 'Try again'}
      </Button>
    </div>
  );
}
