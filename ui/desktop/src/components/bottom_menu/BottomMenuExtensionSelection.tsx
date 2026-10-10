import * as React from 'react';
import { useCallback, useEffect, useMemo, useState, useRef } from 'react';
import { Puzzle } from '../icons/app-icons';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import BuiltInBadge from '../ui/BuiltInBadge';
import { FixedExtensionEntry, useConfig } from '../ConfigContext';
import { isCapabilityExtension } from '../settings/capabilities/capabilities';
import { toastService } from '../../toasts';
import {
  formatExtensionName,
  isBuiltInExtension,
} from '../settings/extensions/subcomponents/ExtensionList';
import { ExtensionConfig, getSessionExtensions } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import type { SessionClassification } from '../../api/types.gen';
import { addToAgent, removeFromAgent } from '../settings/extensions/agent-api';
import { extensionPairingRefused } from '../settings/extensions/extensionPrivacy';
import { isCrewExtensionName, useChatCrewAccessState } from '../crew/access/chatCrewAccess';
import { accessCopy } from '../crew/access/copy';
import { useBoundProviderTier } from '../privacy/useBoundProviderTier';
import { setExtensionOverride, getExtensionOverrides } from '../../store/extensionOverrides';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';
import {
  OPEN_CURRENT_CHAT_EXTENSIONS_EVENT,
  type SessionToolEventDetail,
} from '../../utils/sessionToolEvents';
import { EXTENSIONS_COPY, TOOLS_COPY } from './copy';
import { PickerPanel, PickerRow } from './PickerPanel';

interface BottomMenuExtensionSelectionProps {
  sessionId: string | null;
  /**
   * The focused chat's privacy tier (issue #56, §14.5).
   *
   * ⚠ **Not what the pairing is judged on — see `pairingTier` in the hook.**
   * The chat's classification and the bound model's tier are different axes, and
   * feeding this one to `extensionPairingRefused` reported a Private UCSF model
   * as public in every chat that had not yet run a turn.
   */
  privacyTier?: SessionClassification;
}

/** What the extensions list needs from its picker state (see `useExtensionPicker`). */
export interface ExtensionPicker {
  sessionId: string | null;
  /** Home: a change shapes the next chat only. */
  isHubView: boolean;
  /** The chip's number: extensions the user added, shipped capabilities excluded. */
  activeCount: number;
  query: string;
  setQuery: (query: string) => void;
  rows: ExtensionRow[];
  toggle: (ext: FixedExtensionEntry, next?: boolean) => void;
  bulk: { label: string; onClick: () => void; disabled: boolean } | null;
}

export interface ExtensionRow {
  ext: FixedExtensionEntry;
  pairingRefused: boolean;
  lockedByCrew: boolean;
  disabled: boolean;
  pending: boolean;
}

/**
 * The composer's extension list: state, fetches and toggles, without any
 * trigger. `ToolsChip` holds it for the whole time the composer is mounted, so
 * the count on the chip is right before the list is ever opened.
 *
 * @param open whether the list is on screen; opening re-reads the chat's list.
 * @param onOpenRequest called when the app asks to show this chat's extensions
 *   (`OPEN_CURRENT_CHAT_EXTENSIONS_EVENT`).
 */
export function useExtensionPicker(
  sessionId: string | null,
  { open, onOpenRequest }: { open: boolean; onOpenRequest?: () => void }
): ExtensionPicker {
  /**
   * The tier this surface judges a pairing on: the **bound model's**, resolved
   * from a live instance (issue #56).
   *
   * ⚠ **It is deliberately NOT the session's classification.** Gate C asks
   * `privacy_refusal(extension, extension_tier, cap.tier())` where `cap.tier()`
   * is `Provider::tier()` off the bound instance — the session row is not
   * consulted at all. This surface previously passed the session's ratcheted
   * `privacy_tier`, which is `public` until the first turn raises it, so a chat
   * on UCSF Versa (Private) rendered `ucsfomopagent` and `cdwagent` disabled
   * with "Unavailable in this chat (public model)" while the daemon would have
   * dispatched them. `extensionPairingRefused`'s doc claims it matches
   * `privacy_refusal` "exactly"; that is true of the rule and was false of the
   * argument, which is what let the mismatch through.
   *
   * `undefined` while unresolved, and `extensionPairingRefused` judges nothing
   * on it — a tier nobody could read must not wall a working tool.
   */
  const pairingTier = useBoundProviderTier();
  // A Crew-scoped chat's access ends only through Revoke. Switching the Crew extension off leaves
  // the grant active, so while it is active the Crew row stays on and says where Revoke is. Read
  // from the chat's own lookup; this menu never fetches it.
  const crewAccessState = useChatCrewAccessState(sessionId);
  const [searchQuery, setSearchQuery] = useState('');
  const isOpen = open;
  const onOpenRequestRef = useRef(onOpenRequest);
  useEffect(() => {
    onOpenRequestRef.current = onOpenRequest;
  }, [onOpenRequest]);
  const [sessionExtensions, setSessionExtensions] = useState<ExtensionConfig[]>([]);
  const [sessionExtensionsLoaded, setSessionExtensionsLoaded] = useState(false);
  const [hubUpdateTrigger, setHubUpdateTrigger] = useState(0);
  const [sessionOverrides, setSessionOverrides] = useState<Map<string, boolean>>(new Map());
  const [pendingExtensionNames, setPendingExtensionNames] = useState<Set<string>>(new Set());
  const [bulkInFlight, setBulkInFlight] = useState(false);
  const [refreshTrigger, setRefreshTrigger] = useState(0);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const sessionToggleChainsRef = useRef<Map<string, Promise<boolean>>>(new Map());
  const { extensionsList: allExtensions } = useConfig();
  const isHubView = !sessionId;

  useEffect(() => {
    const handleSessionLoaded = () => {
      if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
      refreshTimerRef.current = setTimeout(() => {
        refreshTimerRef.current = null;
        setRefreshTrigger((prev) => prev + 1);
      }, 500);
    };

    window.addEventListener('session-created', handleSessionLoaded);
    window.addEventListener('message-stream-finished', handleSessionLoaded);
    // Issue #112. The global inventory is repaired by `ConfigContext`, but this
    // menu also holds the SESSION's own list — the thing that decides whether a
    // row reads as on for this chat — and a catalogue change can move that too
    // (an agent-driven install attaches as it goes). Same debounce as the two
    // above, deliberately: they are the same kind of "something changed
    // underneath us" signal.
    window.addEventListener(CATALOG_CHANGED_EVENT, handleSessionLoaded);

    return () => {
      window.removeEventListener('session-created', handleSessionLoaded);
      window.removeEventListener('message-stream-finished', handleSessionLoaded);
      window.removeEventListener(CATALOG_CHANGED_EVENT, handleSessionLoaded);
      if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
    };
  }, []);

  useEffect(() => {
    const openForCurrentChat = (event: Event) => {
      const detail = (event as CustomEvent<SessionToolEventDetail>).detail;
      if (detail?.sessionId === sessionId) onOpenRequestRef.current?.();
    };
    window.addEventListener(OPEN_CURRENT_CHAT_EXTENSIONS_EVENT, openForCurrentChat);
    return () => window.removeEventListener(OPEN_CURRENT_CHAT_EXTENSIONS_EVENT, openForCurrentChat);
  }, [sessionId]);

  useEffect(() => {
    sessionToggleChainsRef.current.clear();
    setSessionExtensions([]);
    setSessionExtensionsLoaded(false);
    setSessionOverrides(new Map());
    setPendingExtensionNames(new Set());
  }, [sessionId]);

  // Fetch session-specific extensions or use global defaults
  useEffect(() => {
    let current = true;

    const fetchExtensions = async () => {
      if (!sessionId) {
        return;
      }

      try {
        // With the user's proof: a private chat's extensions are refused to a
        // caller without it (issue #56, QA 2026-09-10).
        const response = await getSessionExtensions({
          path: { session_id: sessionId },
          headers: await userActionHeaders(),
        });

        if (current && response.data?.extensions) {
          setSessionExtensions(response.data.extensions);
          setSessionExtensionsLoaded(true);
        }
      } catch (error) {
        console.error('Failed to fetch session extensions:', error);
      }
    };

    fetchExtensions();

    return () => {
      current = false;
    };
  }, [sessionId, isOpen, refreshTrigger]);

  const handleToggle = useCallback(
    (extensionConfig: FixedExtensionEntry, requestedState?: boolean) => {
      const nextEnabled = requestedState ?? !extensionConfig.enabled;
      // No success toast (decision 6.9): the switch is the confirmation, and
      // the scope line's InfoTip says how far the change reaches. On Home,
      // `extensionOverrides` is an in-memory map that `createSession` clears,
      // so it shapes exactly the next chat.
      if (isHubView) {
        setExtensionOverride(extensionConfig.name, nextEnabled);
        setHubUpdateTrigger((prev) => prev + 1);
        return;
      }

      if (!sessionId) {
        toastService.error({
          title: EXTENSIONS_COPY.toggleErrorTitle,
          msg: EXTENSIONS_COPY.startChatFirst,
          traceback: 'No session ID available',
        });
        return;
      }

      const name = extensionConfig.name;
      setSessionOverrides((prev) => new Map(prev).set(name, nextEnabled));
      setPendingExtensionNames((prev) => new Set(prev).add(name));

      const previous =
        sessionToggleChainsRef.current.get(name) ?? Promise.resolve(extensionConfig.enabled);
      const operation = previous.then(async (appliedState) => {
        if (appliedState === nextEnabled) return appliedState;
        try {
          if (nextEnabled) {
            await addToAgent(extensionConfig, sessionId, true);
          } else {
            await removeFromAgent(name, sessionId, true);
          }
          return nextEnabled;
        } catch {
          toastService.error({
            title: EXTENSIONS_COPY.toggleErrorTitle,
            msg: EXTENSIONS_COPY.toggleFailed(formatExtensionName(name), nextEnabled),
          });
          return appliedState;
        }
      });

      sessionToggleChainsRef.current.set(name, operation);
      void operation.then(async () => {
        if (sessionToggleChainsRef.current.get(name) !== operation) return;

        try {
          const response = await getSessionExtensions({
            path: { session_id: sessionId },
            headers: await userActionHeaders(),
          });
          if (sessionToggleChainsRef.current.get(name) !== operation) return;
          if (response.data?.extensions) {
            setSessionExtensions(response.data.extensions);
            setSessionExtensionsLoaded(true);
          }
        } catch {
          toastService.error({
            title: EXTENSIONS_COPY.refreshErrorTitle,
            msg: EXTENSIONS_COPY.refreshFailed,
          });
        } finally {
          if (sessionToggleChainsRef.current.get(name) === operation) {
            sessionToggleChainsRef.current.delete(name);
            setSessionOverrides((prev) => {
              const next = new Map(prev);
              next.delete(name);
              return next;
            });
            setPendingExtensionNames((prev) => {
              const next = new Set(prev);
              next.delete(name);
              return next;
            });
          }
        }
      });
    },
    [isHubView, sessionId]
  );

  // Merge all available extensions with session-specific or hub override state
  const extensionsList = useMemo(() => {
    const hubOverrides = getExtensionOverrides();

    // Shipped capabilities are managed in Settings → Chat → Capabilities, not
    // overridden per conversation. The chat menu contains only user-installed
    // extensions.
    const togglable = allExtensions.filter((ext) => !isCapabilityExtension(ext));

    if (isHubView) {
      return togglable.map(
        (ext) =>
          ({
            ...ext,
            enabled: hubOverrides.has(ext.name) ? hubOverrides.get(ext.name)! : ext.enabled,
          }) as FixedExtensionEntry
      );
    }

    const sessionExtensionNames = new Set(sessionExtensions.map((ext) => ext.name));

    return togglable.map(
      (ext) =>
        ({
          ...ext,
          enabled: sessionOverrides.has(ext.name)
            ? sessionOverrides.get(ext.name)!
            : sessionExtensionNames.has(ext.name),
        }) as FixedExtensionEntry
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [allExtensions, sessionExtensions, sessionOverrides, isHubView, hubUpdateTrigger]);

  const filteredExtensions = useMemo(() => {
    return extensionsList.filter((ext) => {
      const query = searchQuery.toLowerCase();
      return (
        ext.name.toLowerCase().includes(query) ||
        (ext.description && ext.description.toLowerCase().includes(query))
      );
    });
  }, [extensionsList, searchQuery]);

  const sortedExtensions = useMemo(() => {
    return [...filteredExtensions].sort((a, b) => a.name.localeCompare(b.name));
  }, [filteredExtensions]);

  /**
   * The rows "Enable all" may actually act on.
   *
   * A refused pairing is rendered but not toggleable, so it must not be counted
   * in the bulk label either — otherwise "Enable all (4)" enables three and
   * leaves the fourth looking broken, which is the failure this state exists to
   * remove.
   */
  const crewLocked = useCallback(
    (ext: FixedExtensionEntry) =>
      crewAccessState === 'active' && ext.enabled && isCrewExtensionName(ext.name),
    [crewAccessState]
  );
  const toggleableExtensions = useMemo(
    () =>
      sortedExtensions.filter(
        (ext) => !extensionPairingRefused(ext.name, pairingTier) && !crewLocked(ext)
      ),
    [sortedExtensions, pairingTier, crewLocked]
  );

  /**
   * The chip's number: how many extensions **the user added** to this chat.
   *
   * Two independent decisions, and they are easy to conflate:
   *
   * 1. *What is counted* — user-installed extensions only. Shipped capabilities
   *    (Developer, Biorouter Copilot, Auto Visualiser, Memory, Knowledge,
   *    Agent Drafter, Todo, Chat Recall, Extension Manager, Code Execution,
   *    Skills) are excluded; see the note inside. A chat whose only extensions
   *    are capabilities therefore reads `0`, and that is the number meaning
   *    "you have added none", not the v1.89.0 P-04 defect wearing the same face.
   * 2. *Where the number comes from* — the SESSION's extensions, which is what
   *    the agent holds and what History's own per-session "N extensions" column
   *    shows. **Not the enabled state of `extensionsList`**, which is the menu's
   *    config-derived rows: an extension can be listed here and not attached to
   *    this chat, or attached here while off in the config, and the chip must
   *    follow the chat. That half is P-04's fix and it stands.
   *
   * Before the session fetch lands there is nothing session-specific to count,
   * so it falls back to the enabled config — the same fallback `GET
   * /sessions/{id}/extensions` applies server-side, so the chip does not flash
   * `0` on the way to the real number. `sessionOverrides` is layered on top so a
   * toggle moves the chip immediately rather than after the refetch.
   */
  const activeCount = useMemo(() => {
    const hubOverrides = getExtensionOverrides();

    // ⚠ SHIPPED CAPABILITIES DO NOT COUNT. They come with the app, they are
    // managed in Settings → Chat → Capabilities rather than here, and every
    // chat has them — so counting them makes the chip a constant with a small
    // number added, which is not what a reader takes it to mean. The number is
    // "extensions I added", and the exclusion uses `isCapabilityExtension`, the
    // same predicate the menu already filters its own contents by, so the chip
    // and the list it labels can never disagree about what an extension is.
    //
    // Session data may arrive before the global catalog; capability identity
    // must not depend on that fetch completing.
    const isShippedCapability = (name: string) => isCapabilityExtension({ name });

    if (isHubView) {
      return allExtensions.filter(
        (ext) =>
          !isCapabilityExtension(ext) &&
          (hubOverrides.has(ext.name) ? hubOverrides.get(ext.name)! : ext.enabled)
      ).length;
    }

    const enabled = new Set(
      sessionExtensionsLoaded
        ? sessionExtensions.map((ext) => ext.name)
        : allExtensions.filter((ext) => ext.enabled).map((ext) => ext.name)
    );
    sessionOverrides.forEach((isEnabled, name) => {
      if (isEnabled) enabled.add(name);
      else enabled.delete(name);
    });
    return [...enabled].filter((name) => !isShippedCapability(name)).length;
    // hubUpdateTrigger re-reads the hub override map, which mutates in place.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    allExtensions,
    sessionExtensions,
    sessionExtensionsLoaded,
    sessionOverrides,
    isHubView,
    hubUpdateTrigger,
  ]);

  const visibleEnabledCount = useMemo(
    () => toggleableExtensions.filter((ext) => ext.enabled).length,
    [toggleableExtensions]
  );

  const handleBulkToggle = useCallback(async () => {
    if (bulkInFlight || pendingExtensionNames.size > 0 || toggleableExtensions.length === 0) {
      return;
    }

    const targetEnabled = visibleEnabledCount === 0;
    const targets = toggleableExtensions.filter((ext) => ext.enabled !== targetEnabled);
    if (targets.length === 0) {
      return;
    }

    setBulkInFlight(true);
    if (isHubView) {
      targets.forEach((ext) => setExtensionOverride(ext.name, targetEnabled));
      setHubUpdateTrigger((prev) => prev + 1);
      setBulkInFlight(false);
      return;
    }

    if (!sessionId) {
      setBulkInFlight(false);
      toastService.error({
        title: EXTENSIONS_COPY.toggleErrorTitle,
        msg: EXTENSIONS_COPY.startChatFirst,
        traceback: 'No session ID available',
      });
      return;
    }

    try {
      setSessionOverrides((prev) => {
        const next = new Map(prev);
        targets.forEach((ext) => next.set(ext.name, targetEnabled));
        return next;
      });
      await Promise.all(
        targets.map((ext) =>
          targetEnabled
            ? addToAgent(ext, sessionId, true)
            : removeFromAgent(ext.name, sessionId, true)
        )
      );
      const response = await getSessionExtensions({
        path: { session_id: sessionId },
        headers: await userActionHeaders(),
      });
      if (response.data?.extensions) {
        setSessionExtensions(response.data.extensions);
        setSessionExtensionsLoaded(true);
      }
    } catch {
      toastService.error({
        title: EXTENSIONS_COPY.toggleErrorTitle,
        msg: EXTENSIONS_COPY.bulkFailed,
      });
    } finally {
      setSessionOverrides((prev) => {
        const next = new Map(prev);
        targets.forEach((ext) => next.delete(ext.name));
        return next;
      });
      setBulkInFlight(false);
    }
  }, [
    bulkInFlight,
    pendingExtensionNames.size,
    toggleableExtensions,
    visibleEnabledCount,
    isHubView,
    sessionId,
  ]);

  const rows = useMemo<ExtensionRow[]>(
    () =>
      sortedExtensions.map((ext) => {
        // §14.5, and the reason it is a *state* rather than an omission:
        // dropping the row is what produces "the OMOP tool is broken". Gate C
        // is invisible in the GUI by construction — it returns `ErrorData` from
        // inside `dispatch_tool_call`, so nothing else in this app will ever
        // tell the user why the tool did nothing. The row stays, disabled, and
        // says why on a visible second line.
        const pairingRefused = extensionPairingRefused(ext.name, pairingTier);
        const lockedByCrew = crewLocked(ext);
        return {
          ext,
          pairingRefused,
          lockedByCrew,
          disabled: bulkInFlight || pairingRefused || lockedByCrew,
          pending: pendingExtensionNames.has(ext.name),
        };
      }),
    [sortedExtensions, pairingTier, crewLocked, bulkInFlight, pendingExtensionNames]
  );

  const bulk =
    toggleableExtensions.length > 0
      ? {
          label:
            visibleEnabledCount === 0
              ? EXTENSIONS_COPY.enableAll(toggleableExtensions.length)
              : EXTENSIONS_COPY.disableAll(visibleEnabledCount),
          onClick: () => void handleBulkToggle(),
          disabled: bulkInFlight || pendingExtensionNames.size > 0,
        }
      : null;

  return {
    sessionId,
    isHubView,
    activeCount,
    query: searchQuery,
    setQuery: setSearchQuery,
    rows,
    toggle: handleToggle,
    bulk,
  };
}

/** The extensions list inside the Tools popover. */
export function ExtensionsPanel({
  picker,
  inputRef,
}: {
  picker: ExtensionPicker;
  inputRef?: React.Ref<HTMLInputElement>;
}) {
  return (
    <PickerPanel
      data-testid="tools-extensions-panel"
      listLabel={EXTENSIONS_COPY.listLabel}
      searchPlaceholder={EXTENSIONS_COPY.search}
      query={picker.query}
      onQueryChange={picker.setQuery}
      scopeLabel={picker.isHubView ? TOOLS_COPY.scope.nextChat : TOOLS_COPY.scope.thisChat}
      scopeHelp={picker.isHubView ? EXTENSIONS_COPY.helpNextChat : EXTENSIONS_COPY.helpChat}
      bulk={picker.bulk}
      emptyText={picker.query ? EXTENSIONS_COPY.noMatch : EXTENSIONS_COPY.empty}
      inputRef={inputRef}
    >
      {picker.rows.map(({ ext, pairingRefused, lockedByCrew, disabled, pending }) => (
        <PickerRow
          key={ext.name}
          name={formatExtensionName(ext.name)}
          checked={ext.enabled}
          disabled={disabled}
          busy={pending}
          onToggle={() => picker.toggle(ext, !ext.enabled)}
          badge={isBuiltInExtension(ext) ? <BuiltInBadge /> : undefined}
          note={
            pairingRefused
              ? EXTENSIONS_COPY.pairingRefused
              : lockedByCrew
                ? accessCopy.extensionLocked
                : undefined
          }
          description={
            pairingRefused
              ? EXTENSIONS_COPY.pairingRefusedReason
              : lockedByCrew
                ? undefined
                : ext.description || undefined
          }
        />
      ))}
    </PickerPanel>
  );
}

/**
 * ⚠ Transitional: the composer's old standalone extensions chip. The app no
 * longer renders it (`ToolsChip` replaced it, spec 3.7); its one importer is
 * `crew/access/crewExtensionLock.test.tsx`, which WS-PRIMITIVES moves onto
 * `useExtensionPicker` + `ExtensionsPanel` (request WS-PICKERS-1). Delete it
 * then.
 */
export function BottomMenuExtensionSelection({ sessionId }: BottomMenuExtensionSelectionProps) {
  const [open, setOpen] = useState(false);
  const picker = useExtensionPicker(sessionId, { open, onOpenRequest: () => setOpen(true) });
  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (!next) picker.setQuery('');
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <button
              type="button"
              className="br-picker-chip"
              aria-label={`Manage extensions (${picker.activeCount} added)`}
            >
              <Puzzle className="br-picker-chip__chevron" aria-hidden="true" />
              <span className="br-picker-chip__count">{picker.activeCount}</span>
            </button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">{EXTENSIONS_COPY.listLabel}</TooltipContent>
      </Tooltip>
      <PopoverContent side="top" align="center" className="br-picker-popover">
        <ExtensionsPanel picker={picker} />
      </PopoverContent>
    </Popover>
  );
}
