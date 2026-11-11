import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { ProviderDetails } from '../../../../api';
import { subscribeAppModelSelectionChanges } from '../../../../utils/sessionBindingSync';
import { useConfig } from '../../../ConfigContext';
import { Button } from '../../../ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '../../../ui/popover';
import { SettingRow } from '../../../ui/setting-row';
import {
  CrewModelList,
  ModelTierMarks,
  useProviderModels,
} from '../../../crew/pane/CrewModelPicker';
import { modelDisplay } from '../../../crew/pane/presentation';
import { isDecisionProvider } from '../../../decisions/decisionWire';
import { defaultWorkerModelCopy } from '../copy';
import { joinModelValue } from '../defaultModelValue';

/**
 * The provider half of the default worker model (`agents::default_worker` in the core crate). The
 * daemon reads both keys per spawn and treats a half-written pair as unset, so the row writes the
 * model first and the provider last, and clears the provider first.
 */
export const WORKER_PROVIDER_KEY = 'BIOROUTER_WORKER_PROVIDER';
/** The model half of the default worker model. */
export const WORKER_MODEL_KEY = 'BIOROUTER_WORKER_MODEL';

/** A provider and one of its models. */
export interface WorkerChoice {
  provider: string;
  model: string;
}

const text = (value: unknown): string => (typeof value === 'string' ? value.trim() : '');

/** The stored default worker model, or `null` unless both halves are non-blank strings. */
export function parseWorkerChoice(provider: unknown, model: unknown): WorkerChoice | null {
  const p = text(provider);
  const m = text(model);
  return p && m ? { provider: p, model: m } : null;
}

/**
 * Whether a configured provider can run a subagent. A decision model cannot: it answers with a
 * choice and its probabilities, so the daemon would run the subagent on the chat's model instead.
 */
export function workerProviderUsable(provider: ProviderDetails | undefined): boolean {
  return (
    provider !== undefined && provider.is_configured === true && !isDecisionProvider(provider.name)
  );
}

interface Read {
  providers: ProviderDetails[];
  provider: unknown;
  model: unknown;
}

const failureText = (failure: unknown) =>
  failure instanceof Error && failure.message ? failure.message : String(failure);

/**
 * Settings › Models › Default model › **Default worker model**: the model a subagent runs on when
 * the agent names none. "Same as the chat’s model" while it is unset, which is how subagents ran
 * before the setting existed; else the model with its tier mark. **Change…** opens a picker of the
 * configured chat providers with "Same as the chat’s model" first.
 *
 * It replaced lead/worker mode (owner, 2026-11-11), and it is a preference, not a capability key:
 * a subagent always runs at its chat's privacy tier, and a default the daemon may not use for a
 * chat (a public model under a private chat, or the reverse) runs that chat's subagents on the
 * chat's own model and says so to the agent. So the row is live on a browser surface too, like the
 * Crew tasks row beside it.
 */
export function DefaultWorkerModelRow() {
  const config = useConfig();
  const { getProviders, read } = config;
  const [state, setState] = useState<Read | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const ticket = useRef(0);

  const load = useCallback(async () => {
    const mine = ++ticket.current;
    try {
      const [items, provider, model] = await Promise.all([
        getProviders(false),
        read(WORKER_PROVIDER_KEY, false),
        read(WORKER_MODEL_KEY, false),
      ]);
      if (mine !== ticket.current) return;
      const providers = (Array.isArray(items) ? items : []).filter(
        (item): item is ProviderDetails =>
          Boolean(item) && typeof item.name === 'string' && item.is_configured === true
      );
      setState({ providers, provider, model });
    } catch (error) {
      if (mine === ticket.current) console.error('Could not read the default worker model:', error);
    }
  }, [getProviders, read]);

  useEffect(() => {
    void load();
    return subscribeAppModelSelectionChanges(() => void load());
  }, [load]);

  const stored = state ? parseWorkerChoice(state.provider, state.model) : null;
  const providerOf = (choice: WorkerChoice) =>
    state?.providers.find((item) => item.name === choice.provider);
  const own = stored && workerProviderUsable(providerOf(stored)) ? stored : null;
  const named = (choice: WorkerChoice) => {
    const shown = modelDisplay(choice, providerOf(choice));
    return joinModelValue(shown.model, shown.provider);
  };

  let value: ReactNode = undefined;
  if (state) {
    value = own ? (
      <span className="inline-flex min-w-0 flex-wrap items-center justify-end gap-1.5">
        <span className="min-w-0 break-words">{named(own)}</span>
        <ModelTierMarks provider={providerOf(own)} />
      </span>
    ) : (
      defaultWorkerModelCopy.sameAsChat
    );
  }
  // A stored model whose provider is no longer set up is not the one subagents run on, and the
  // row says so rather than reading "Same as the chat's model" alone.
  const status =
    failure ?? (stored && !own ? defaultWorkerModelCopy.unavailable(named(stored)) : null);

  const choose = async (choice: WorkerChoice | null) => {
    setFailure(null);
    try {
      if (choice) {
        // The provider last: until it lands the daemon reads the pair as unset and a subagent
        // runs on the chat's model, never on one provider paired with another provider's model.
        if (stored) await config.remove(WORKER_PROVIDER_KEY, false);
        await config.upsert(WORKER_MODEL_KEY, choice.model, false);
        await config.upsert(WORKER_PROVIDER_KEY, choice.provider, false);
      } else {
        await config.remove(WORKER_PROVIDER_KEY, false);
        await config.remove(WORKER_MODEL_KEY, false);
      }
    } catch (error) {
      setFailure(defaultWorkerModelCopy.saveFailed(failureText(error)));
    }
    // Whatever happened, say what the daemon now has.
    await load();
  };

  return (
    <SettingRow
      label={defaultWorkerModelCopy.row}
      help={defaultWorkerModelCopy.help}
      value={value}
      status={status}
    >
      <DefaultWorkerModelControl
        providers={state?.providers ?? null}
        current={own}
        onChoose={(choice) => void choose(choice)}
      />
    </SettingRow>
  );
}

interface DefaultWorkerModelControlProps {
  /** The configured providers, or `null` until they are read. */
  providers: ProviderDetails[] | null;
  /** The default worker model, or `null` for "Same as the chat's model". */
  current: WorkerChoice | null;
  /** A model, or `null` for "Same as the chat's model". */
  onChoose(choice: WorkerChoice | null): void;
  /** Given by `SettingRow`, which names and describes its one control. */
  id?: string;
  'aria-describedby'?: string;
}

/**
 * The row's one control: **Change…**, named "Choose the default worker model", which opens the
 * picker in a popover. The row's id and description go on the button, the element they are about.
 */
function DefaultWorkerModelControl({
  providers,
  current,
  onChoose,
  id,
  'aria-describedby': describedBy,
}: DefaultWorkerModelControlProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const close = (next: boolean) => {
    if (!next) setQuery('');
    setOpen(next);
  };
  const pick = (choice: WorkerChoice | null) => {
    close(false);
    onChoose(choice);
  };
  return (
    <Popover open={open} onOpenChange={close}>
      <PopoverTrigger asChild>
        <Button
          id={id}
          variant="secondary"
          size="sm"
          aria-label={defaultWorkerModelCopy.changeName}
          aria-describedby={describedBy}
          disabled={providers === null}
        >
          {defaultWorkerModelCopy.change}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={6}
        className="w-80 p-0"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <DefaultWorkerModelMenu
          providers={providers}
          current={current}
          query={query}
          onQueryChange={setQuery}
          onPick={pick}
        />
      </PopoverContent>
    </Popover>
  );
}

/** The open picker: mounted with the popover, so the models are read only when asked for. */
function DefaultWorkerModelMenu({
  providers,
  current,
  query,
  onQueryChange,
  onPick,
}: {
  providers: ProviderDetails[] | null;
  current: WorkerChoice | null;
  query: string;
  onQueryChange(query: string): void;
  onPick(choice: WorkerChoice | null): void;
}) {
  const usable = useMemo(
    () => (providers === null ? null : providers.filter((item) => workerProviderUsable(item))),
    [providers]
  );
  const models = useProviderModels(usable);
  return (
    <CrewModelList
      label={defaultWorkerModelCopy.listLabel}
      providers={usable}
      models={models}
      provider={current?.provider ?? ''}
      model={current?.model ?? ''}
      query={query}
      onQueryChange={onQueryChange}
      onChoose={(choice) => onPick(choice)}
      leading={{
        label: defaultWorkerModelCopy.sameAsChat,
        selected: current === null,
        onSelect: () => onPick(null),
      }}
    />
  );
}
