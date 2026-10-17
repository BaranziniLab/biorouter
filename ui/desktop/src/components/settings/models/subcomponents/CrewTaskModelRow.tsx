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
import {
  CREW_TASK_MODEL_KEY,
  crewTaskModelUsable,
  parseTaskModel,
  resolveTaskModel,
  type TaskModelChoice,
} from '../../../crew/state/crewTaskModel';
import { crewTaskModelCopy } from '../copy';
import { joinModelValue } from '../defaultModelValue';

/** What the row read from the daemon: the configured providers and the three keys it resolves. */
interface Read {
  providers: ProviderDetails[];
  crew: unknown;
  appProvider: unknown;
  appModel: unknown;
}

const failureText = (failure: unknown) =>
  failure instanceof Error && failure.message ? failure.message : String(failure);

/**
 * Settings › Models › Default model › **Crew tasks** (design OW-11): the model Ask my agent and
 * `@agent` start a Crew task on. "Same as new chats" while `CREW_TASK_MODEL` is unset, else the
 * model with its tier mark; **Change…** opens a picker of the configured providers that can run a
 * Crew task (never Claude Code or Codex, which the daemon refuses for every Crew task) with "Same
 * as new chats" first.
 *
 * A model is written with `upsert('CREW_TASK_MODEL', {provider, model})` and "Same as new chats"
 * removes the key, each announced to every window (`ConfigContext`), so an open Crew pane or
 * composer states the new model at once. It is a preference, not a capability key: every task still
 * names its model and is admitted by the daemon and the broker, so the row is not disabled on a
 * browser surface either.
 *
 * Nothing is drawn as the value until the daemon has answered (never a skeleton: the section's
 * first row paints at rest, V4-R7-1), and the picker's models are read only once it is open.
 */
export function CrewTaskModelRow() {
  const config = useConfig();
  const { getProviders, read } = config;
  const [state, setState] = useState<Read | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const ticket = useRef(0);

  const load = useCallback(async () => {
    const mine = ++ticket.current;
    try {
      const [items, crew, appProvider, appModel] = await Promise.all([
        getProviders(false),
        read(CREW_TASK_MODEL_KEY, false),
        read('BIOROUTER_PROVIDER', false),
        read('BIOROUTER_MODEL', false),
      ]);
      if (mine !== ticket.current) return;
      const providers = (Array.isArray(items) ? items : []).filter(
        (item): item is ProviderDetails =>
          Boolean(item) && typeof item.name === 'string' && item.is_configured === true
      );
      setState({ providers, crew, appProvider, appModel });
    } catch (error) {
      if (mine === ticket.current) console.error('Could not read the model for Crew tasks:', error);
    }
  }, [getProviders, read]);

  useEffect(() => {
    void load();
    return subscribeAppModelSelectionChanges(() => void load());
  }, [load]);

  const resolved = state
    ? resolveTaskModel({
        crew: state.crew,
        appProvider: state.appProvider,
        appModel: state.appModel,
        configured: state.providers,
      })
    : null;
  const stored = state ? parseTaskModel(state.crew) : null;
  const own = resolved?.source === 'crew' ? resolved.choice : null;
  const providerOf = (choice: TaskModelChoice) =>
    state?.providers.find((item) => item.name === choice.provider);
  const named = (choice: TaskModelChoice) => {
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
      crewTaskModelCopy.sameAsNewChats
    );
  }
  // A stored choice that names a provider no longer set up (or one Crew refuses) is not the one
  // tasks start on, and the row says so rather than reading "Same as new chats" alone.
  const status =
    failure ??
    (stored && resolved?.source !== 'crew' ? crewTaskModelCopy.unavailable(named(stored)) : null);

  const choose = async (choice: TaskModelChoice | null) => {
    setFailure(null);
    try {
      if (choice)
        await config.upsert(
          CREW_TASK_MODEL_KEY,
          { provider: choice.provider, model: choice.model },
          false
        );
      else await config.remove(CREW_TASK_MODEL_KEY, false);
    } catch (error) {
      setFailure(crewTaskModelCopy.saveFailed(failureText(error)));
    }
    // Whatever happened, say what the daemon now has.
    await load();
  };

  return (
    <SettingRow
      label={crewTaskModelCopy.row}
      help={crewTaskModelCopy.help}
      value={value}
      status={status}
    >
      <CrewTaskModelControl
        providers={state?.providers ?? null}
        current={own}
        onChoose={(choice) => void choose(choice)}
      />
    </SettingRow>
  );
}

interface CrewTaskModelControlProps {
  /** The configured providers, or `null` until they are read. */
  providers: ProviderDetails[] | null;
  /** The model Crew tasks start on when it is their own; `null` for "Same as new chats". */
  current: TaskModelChoice | null;
  /** A model, or `null` for "Same as new chats". */
  onChoose(choice: TaskModelChoice | null): void;
  /** Given by `SettingRow`, which names and describes its one control. */
  id?: string;
  'aria-describedby'?: string;
}

/**
 * The row's one control: **Change…**, named "Choose the model for Crew tasks", which opens the
 * picker in a popover. The row's id and description go on the button, the element they are about.
 */
function CrewTaskModelControl({
  providers,
  current,
  onChoose,
  id,
  'aria-describedby': describedBy,
}: CrewTaskModelControlProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const close = (next: boolean) => {
    if (!next) setQuery('');
    setOpen(next);
  };
  const pick = (choice: TaskModelChoice | null) => {
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
          aria-label={crewTaskModelCopy.changeName}
          aria-describedby={describedBy}
          disabled={providers === null}
        >
          {crewTaskModelCopy.change}
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        sideOffset={6}
        className="w-80 p-0"
        onOpenAutoFocus={(event) => event.preventDefault()}
      >
        <CrewTaskModelMenu
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
function CrewTaskModelMenu({
  providers,
  current,
  query,
  onQueryChange,
  onPick,
}: {
  providers: ProviderDetails[] | null;
  current: TaskModelChoice | null;
  query: string;
  onQueryChange(query: string): void;
  onPick(choice: TaskModelChoice | null): void;
}) {
  const usable = useMemo(
    () => (providers === null ? null : providers.filter((item) => crewTaskModelUsable(item))),
    [providers]
  );
  const models = useProviderModels(usable);
  return (
    <CrewModelList
      label={crewTaskModelCopy.listLabel}
      providers={usable}
      models={models}
      provider={current?.provider ?? ''}
      model={current?.model ?? ''}
      query={query}
      onQueryChange={onQueryChange}
      onChoose={(choice) => onPick(choice)}
      leading={{
        label: crewTaskModelCopy.sameAsNewChats,
        selected: current === null,
        onSelect: () => onPick(null),
      }}
    />
  );
}
