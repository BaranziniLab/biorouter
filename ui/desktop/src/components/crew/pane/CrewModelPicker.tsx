import { forwardRef, useEffect, useId, useMemo, useState } from 'react';
import type { ProviderDetails } from '../../../api';
import { Check, ChevronDown } from '../../icons/app-icons';
import { useConfig } from '../../ConfigContext';
import {
  affiliationPresentation,
  readProviderAffiliation,
} from '../../privacy/providerAffiliation';
import { AffiliationBadge } from '../../ui/AffiliationBadge';
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '../../ui/command';
import { Popover, PopoverContent, PopoverTrigger } from '../../ui/popover';
import { PrivacyBadge } from '../../ui/PrivacyBadge';
import { cn } from '../../../utils';
import { agentCopy } from './copy';
import { modelDisplay } from './presentation';
import { providerLabel, type ModelChoice } from './useConfiguredModels';
import './pane.css';

/**
 * A model's tier and who approved it, as ONE mark with words: "🔒 Private · UCSF", or "Public"
 * (Q2-67). It used to be two bare glyphs — the dense padlock and the dense affiliation mark — that
 * a person could not read, under a tooltip about "this chat".
 *
 * The padlock pill is `PrivacyBadge`, the app's one mark for the private tier, followed by the
 * affiliation's words (`affiliationPresentation`, the registry's name for the institution), as the
 * sidebar's privacy chip writes "Private · ucsf". Its `title` says what the tier means for THIS
 * task: "Only private models can run this task." only where the pane knows the task's context is
 * protected (`privateOnly`); elsewhere, what a private model adds.
 *
 * `enforcementOff={false}`, as on the sidebar's chip: the daemon's Crew admission refuses a public
 * model for protected context whatever this machine's privacy master switch says
 * (`crew/institution.rs` `admission` reads no switch), so "(enforcement off)" would be false here.
 */
export function ModelTierMarks({
  provider,
  privateOnly = false,
}: {
  provider: ProviderDetails | undefined;
  /** The task's context is protected, so only a private model can run it. */
  privateOnly?: boolean;
}) {
  const tier = provider?.resolved_tier;
  const affiliation = readProviderAffiliation(provider);
  if (!tier) {
    // A tier the daemon could not resolve says nothing about privacy; an affiliation still names
    // who approved the model.
    return affiliation ? <AffiliationBadge affiliation={affiliation} /> : null;
  }
  const approvedBy = tier === 'private' ? affiliationPresentation(affiliation)?.label : null;
  const title =
    tier === 'public'
      ? agentCopy.publicModel
      : privateOnly
        ? agentCopy.privateOnly
        : agentCopy.privateModel;
  return (
    <span
      className="inline-flex min-w-0 shrink-0 flex-wrap items-center gap-1 text-label"
      title={title}
      data-crew-model-tier={tier}
    >
      <PrivacyBadge tier={tier} enforcementOff={false} />
      {/* The spaces are text, not layout (the flex gap draws the space), so the mark reads and
          copies as "Private · UCSF" and is spoken "Private UCSF". */}
      {approvedBy && (
        <>
          {' '}
          <span aria-hidden="true" className="text-text-muted">
            ·
          </span>{' '}
          <bdi className="text-text-default">{approvedBy}</bdi>
        </>
      )}
    </span>
  );
}

/** The models a provider offers: its curated list, else what the daemon reports for it. */
async function modelsOf(
  provider: ProviderDetails,
  getProviderModels: (name: string) => Promise<string[]>
): Promise<string[]> {
  const known = (provider.metadata?.known_models ?? [])
    .map((model) => model?.name)
    .filter((name): name is string => typeof name === 'string' && name !== '');
  if (known.length > 0) return known;
  try {
    const listed = await getProviderModels(provider.name);
    return (Array.isArray(listed) ? listed : []).filter(
      (name): name is string => typeof name === 'string' && name !== ''
    );
  } catch {
    // A provider whose list cannot be fetched still takes a model typed by name.
    return [];
  }
}

export interface CrewModelPickerProps {
  /** The configured providers to choose from; `null` while they load. */
  providers: readonly ProviderDetails[] | null;
  provider: string;
  model: string;
  onChange(choice: ModelChoice): void;
  open?: boolean;
  onOpenChange?(open: boolean): void;
  /** The id of a visible "Model" label; without one the trigger labels itself "Model". */
  labelledBy?: string;
  /**
   * Why a provider's models cannot start a task here ("Not approved for foreign-synthetic"), or
   * `null`. Its models stay choosable — the pane explains the choice and the daemon still decides —
   * but each is marked, so a person sees it before choosing (T-47).
   */
  unavailableReason?(provider: ProviderDetails): string | null;
  /** The task's context is protected, so each model's mark says only private models can run it. */
  privateOnly?: boolean;
  /** Field validation: the choice is missing. */
  invalid?: boolean;
  describedBy?: string;
  disabled?: boolean;
  className?: string;
}

/**
 * The model picker for Ask my agent (ui-redesign-spec, "Ask my agent"): a field-look `Popover` +
 * `Command` whose trigger is named "Model" followed by its value ("Choose a model" when empty).
 * Models are grouped by configured provider, each heading carrying the provider's tier and
 * affiliation marks, and typing a name that is not listed offers "Use “{text}” with {provider}",
 * which keeps the free text the old field allowed. A provider the workspace's institution has not
 * approved is marked on its heading and on each of its models (`unavailableReason`).
 *
 * It chooses; it does not authorize. The daemon checks the provider's tier and institution against
 * the workspace when the task starts.
 */
export const CrewModelPicker = forwardRef<HTMLButtonElement, CrewModelPickerProps>(
  function CrewModelPicker(
    {
      providers: providersProp,
      provider,
      model,
      onChange,
      open: openProp,
      onOpenChange,
      labelledBy,
      unavailableReason,
      privateOnly = false,
      invalid = false,
      describedBy,
      disabled = false,
      className,
    },
    ref
  ) {
    const { getProviderModels } = useConfig();
    const [openState, setOpenState] = useState(false);
    const open = openProp ?? openState;
    const setOpen = (next: boolean) => {
      if (openProp === undefined) setOpenState(next);
      onOpenChange?.(next);
    };
    const [query, setQuery] = useState('');
    const [models, setModels] = useState<Record<string, string[]> | null>(null);
    const ownLabelId = useId();
    const valueId = useId();
    const providers = useMemo(() => providersProp ?? [], [providersProp]);
    const providersLoading = providersProp === null;

    const providerKey = providers.map((item) => item.name).join('\n');
    useEffect(() => {
      let active = true;
      setModels(null);
      void Promise.all(
        providers.map(async (item) => [item.name, await modelsOf(item, getProviderModels)] as const)
      ).then((entries) => {
        if (active) setModels(Object.fromEntries(entries));
      });
      return () => {
        active = false;
      };
      // `providerKey` stands for the list: a new array with the same providers is the same list.
      // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [providerKey, getProviderModels]);

    const selectedProvider = providers.find((item) => item.name === provider);
    const hasValue = provider !== '' && model !== '';
    const shown = modelDisplay({ provider, model }, selectedProvider);
    const triggerValue = agentCopy.modelChoice(shown.model, shown.provider);
    const needle = query.trim().toLowerCase();
    const groups = useMemo(
      () =>
        providers.map((item) => {
          const label = providerLabel(item);
          const all = models?.[item.name] ?? [];
          const providerMatches =
            needle !== '' &&
            (item.name.toLowerCase().includes(needle) || label.toLowerCase().includes(needle));
          const listed = all.filter(
            (name) => needle === '' || providerMatches || name.toLowerCase().includes(needle)
          );
          const freeText =
            needle !== '' && !all.some((name) => name.toLowerCase() === needle)
              ? query.trim()
              : null;
          return {
            provider: item,
            label,
            listed,
            freeText,
            unavailable: unavailableReason?.(item) ?? null,
          };
        }),
      [providers, models, needle, query, unavailableReason]
    );
    const anyRows = groups.some((group) => group.listed.length > 0 || group.freeText);

    const choose = (choice: ModelChoice) => {
      onChange(choice);
      setQuery('');
      setOpen(false);
    };

    return (
      <Popover
        open={open}
        onOpenChange={(next) => {
          if (!next) setQuery('');
          setOpen(next);
        }}
      >
        <PopoverTrigger asChild>
          <button
            ref={ref}
            type="button"
            disabled={disabled}
            aria-labelledby={`${labelledBy ?? ownLabelId} ${valueId}`}
            aria-invalid={invalid || undefined}
            aria-describedby={describedBy}
            className={cn(
              'crew-model-trigger biorouter-focus-surface flex min-h-control-md w-full min-w-0 items-center gap-2 rounded-element border border-border-emphasized bg-background-default px-2 py-1 text-left text-label transition-[color,background-color,border-color,box-shadow] hover:inset-ring-2 hover:inset-ring-border-emphasized/30 aria-invalid:border-border-danger disabled:cursor-not-allowed disabled:opacity-50',
              className
            )}
          >
            {!labelledBy && (
              <span id={ownLabelId} className="sr-only">
                {agentCopy.model}
              </span>
            )}
            {/* Wraps rather than ellipsizing: the provider is the part a truncation cut (T-47). */}
            <span
              id={valueId}
              className={cn(
                'min-w-0 flex-1 break-words',
                hasValue ? 'text-text-default' : 'text-text-muted'
              )}
            >
              {hasValue ? triggerValue : agentCopy.modelEmpty}
            </span>
            {hasValue && <ModelTierMarks provider={selectedProvider} privateOnly={privateOnly} />}
            <ChevronDown
              aria-hidden="true"
              className="crew-model-chevron h-icon-row w-icon-row shrink-0 text-text-muted"
            />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          sideOffset={6}
          className="w-80 p-0"
          onOpenAutoFocus={(event) => event.preventDefault()}
        >
          <Command
            label={agentCopy.modelsLabel}
            query={query}
            onQueryChange={setQuery}
            className="crew-model-list"
          >
            <CommandInput
              placeholder={agentCopy.searchModels}
              aria-label={agentCopy.searchModels}
              autoFocus
            />
            <CommandList aria-label={agentCopy.modelsLabel}>
              {models === null || providersLoading ? (
                <CommandEmpty>
                  <p className="text-supporting text-text-muted">{agentCopy.loadingModels}</p>
                </CommandEmpty>
              ) : !anyRows ? (
                <CommandEmpty>
                  <p className="text-supporting text-text-muted">{agentCopy.noMatch}</p>
                </CommandEmpty>
              ) : (
                groups.map((group) =>
                  group.listed.length === 0 && !group.freeText ? null : (
                    <CommandGroup
                      key={group.provider.name}
                      heading={
                        <span className="inline-flex flex-wrap items-center gap-1.5">
                          <span>{group.label}</span>
                          <ModelTierMarks provider={group.provider} privateOnly={privateOnly} />
                          {group.unavailable && (
                            <span className="text-supporting text-text-muted">
                              {group.unavailable}
                            </span>
                          )}
                        </span>
                      }
                    >
                      {group.listed.map((name) => {
                        const selected = provider === group.provider.name && model === name;
                        return (
                          <CommandItem
                            key={name}
                            selected={selected}
                            onSelect={() => choose({ provider: group.provider.name, model: name })}
                          >
                            <span className="flex min-w-0 flex-1 flex-col">
                              <span className="truncate">{name}</span>
                              {group.unavailable && (
                                <span className="text-supporting text-text-muted">
                                  {group.unavailable}
                                </span>
                              )}
                            </span>
                            {selected && (
                              <Check
                                aria-hidden="true"
                                className="h-icon-row w-icon-row shrink-0 text-text-default"
                              />
                            )}
                          </CommandItem>
                        );
                      })}
                      {group.freeText && (
                        <CommandItem
                          onSelect={() =>
                            choose({ provider: group.provider.name, model: group.freeText ?? '' })
                          }
                        >
                          <span className="flex min-w-0 flex-1 flex-col">
                            <span className="truncate">
                              {agentCopy.modelUse(group.freeText, group.label)}
                            </span>
                            {group.unavailable && (
                              <span className="text-supporting text-text-muted">
                                {group.unavailable}
                              </span>
                            )}
                          </span>
                        </CommandItem>
                      )}
                    </CommandGroup>
                  )
                )
              )}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>
    );
  }
);
