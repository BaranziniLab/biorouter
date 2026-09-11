import { useCallback, useEffect, useMemo, useState, useRef } from 'react';
import { Puzzle } from '../icons/app-icons';
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
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
import { useBoundProviderTier } from '../privacy/useBoundProviderTier';
import { setExtensionOverride, getExtensionOverrides } from '../../store/extensionOverrides';
import { CATALOG_CHANGED_EVENT } from '../../utils/catalogSubscription';
import {
  OPEN_CURRENT_CHAT_EXTENSIONS_EVENT,
  type SessionToolEventDetail,
} from '../../utils/sessionToolEvents';

/** §14.5's reason, in the composer's own words. Public model → private tool. */
const PAIRING_REFUSED_REASON =
  'Unavailable in this chat: a private extension needs a private model';

interface BottomMenuExtensionSelectionProps {
  sessionId: string | null;
  /**
   * The focused chat's privacy tier (issue #56, §14.5).
   *
   * ⚠ **No longer what the pairing is judged on — see `pairingTier` in the body.**
   * The chat's classification and the bound model's tier are different axes, and
   * feeding this one to `extensionPairingRefused` reported a Private UCSF model
   * as public in every chat that had not yet run a turn. Kept as a prop because
   * it is still the honest answer to *what tier is this chat*, which is what the
   * model chip beside this one states.
   */
  privacyTier?: SessionClassification;
}

export const BottomMenuExtensionSelection = ({
  sessionId,
  privacyTier: _sessionPrivacyTier,
}: BottomMenuExtensionSelectionProps) => {
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
  const [searchQuery, setSearchQuery] = useState('');
  const [isOpen, setIsOpen] = useState(false);
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
      if (detail?.sessionId === sessionId) setIsOpen(true);
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
      if (isHubView) {
        setExtensionOverride(extensionConfig.name, nextEnabled);
        setHubUpdateTrigger((prev) => prev + 1);

        toastService.success({
          title: 'Extension updated',
          // "in new chats" over-promised: `extensionOverrides` is an in-memory
          // map that `createSession` clears, so it shapes exactly one chat.
          msg: `${formatExtensionName(extensionConfig.name)} will be ${nextEnabled ? 'enabled' : 'disabled'} in the next chat you start`,
        });
        return;
      }

      if (!sessionId) {
        toastService.error({
          title: 'Extension toggle error',
          msg: 'Start a chat first.',
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
            title: 'Extension toggle error',
            msg: `${formatExtensionName(name)} could not be ${nextEnabled ? 'enabled' : 'disabled'}.`,
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
            title: 'Extension refresh error',
            msg: 'The latest extension state could not be refreshed.',
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
  const toggleableExtensions = useMemo(
    () => sortedExtensions.filter((ext) => !extensionPairingRefused(ext.name, pairingTier)),
    [sortedExtensions, pairingTier]
  );

  /**
   * The chip's number: how many extensions **the user added** to this chat.
   *
   * Two independent decisions, and they are easy to conflate:
   *
   * 1. *What is counted* — user-installed extensions only. Shipped capabilities
   *    (Developer, Computer Controller, Auto Visualiser, Memory, Knowledge,
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

      toastService.success({
        title: 'Extensions updated',
        msg: `${targets.length} extension${targets.length === 1 ? '' : 's'} ${targetEnabled ? 'enabled' : 'disabled'} in the next chat you start`,
      });
      return;
    }

    if (!sessionId) {
      setBulkInFlight(false);
      toastService.error({
        title: 'Extension toggle error',
        msg: 'Start a chat first.',
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

      toastService.success({
        title: 'Extensions updated',
        msg: `${targets.length} extension${targets.length === 1 ? '' : 's'} ${targetEnabled ? 'enabled' : 'disabled'} for this chat`,
      });
    } catch {
      toastService.error({
        title: 'Extension toggle error',
        msg: 'The extension selection could not be updated.',
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

  return (
    <DropdownMenu
      open={isOpen}
      onOpenChange={(open) => {
        setIsOpen(open);
        if (!open) {
          setSearchQuery('');
        }
      }}
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              // `text-supporting`, not the `text-secondary` main.css prescribes
              // for a dense control — the override is explained once in
              // ChatInput.tsx, search "THE RAILS' TYPE".
              className="flex h-7 items-center rounded-md px-0.5 cursor-pointer [&_svg]:size-4 text-text-default/70 hover:bg-background-medium hover:text-text-default text-supporting"
              aria-label={`Manage extensions (${activeCount} enabled)`}
            >
              <Puzzle className="mr-0.5 h-4 w-4" />
              <span>{activeCount}</span>
            </button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="top">Manage extensions</TooltipContent>
      </Tooltip>
      <DropdownMenuContent
        side="top"
        align="center"
        className="w-64"
        onCloseAutoFocus={(e) => {
          e.preventDefault();
        }}
      >
        <div className="p-1">
          <Input
            type="text"
            placeholder="Search extensions..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8"
            autoFocus
          />
        </div>
        {/* §4.5 section label names the group; the bulk action shares its row as a
            real ghost affordance (radius + hover fill + hit area) rather than the
            bare underlined text link it used to be. */}
        <div className="flex items-center justify-between gap-2">
          <DropdownMenuLabel>Extensions</DropdownMenuLabel>
          {toggleableExtensions.length > 0 && (
            <button
              type="button"
              onClick={handleBulkToggle}
              disabled={bulkInFlight || pendingExtensionNames.size > 0}
              className="mr-2 shrink-0 cursor-pointer rounded-sm px-1.5 py-0.5 text-chip text-text-muted transition-colors duration-[var(--motion-fast)] hover:bg-background-medium hover:text-text-default disabled:cursor-not-allowed disabled:opacity-50"
            >
              {visibleEnabledCount === 0
                ? `Enable all (${toggleableExtensions.length})`
                : `Disable all (${visibleEnabledCount})`}
            </button>
          )}
        </div>
        {sortedExtensions.length > 0 && (
          /*
           * v1.89.0 P-03. What this menu changes, said where the user is
           * deciding rather than in a toast that has already gone.
           *
           * The control is per-CHAT and always was: a session toggle calls
           * `/agent/add_extension`, which attaches to the running agent and
           * persists into that session's own `enabled_extensions.v0` — so the
           * choice survives a restart *in this chat* — and never touches
           * `config.yaml`. The hub view's toggles are more transient still:
           * `extensionOverrides` is an in-memory map that `createSession`
           * clears, so it shapes exactly the next chat you start.
           *
           * Neither said so. The bulk button's toast did ("… for this chat
           * session"), a single row's said only "Successfully added extension",
           * and nothing at all said where the durable default lives. A user who
           * enabled Workspace here, quit, and found it off again read that as
           * the setting failing to save; it was a per-chat control being read as
           * a global one. Fixing the persistence instead would be the wrong
           * repair — it would make disabling an extension in one chat disable it
           * in every other.
           */
          <div className="px-3 pb-1.5 pt-0.5 text-supporting text-text-muted">
            {isHubView
              ? 'Applies to the next chat you start. Settings → Extensions sets the default for all new chats.'
              : 'Applies to this chat. Settings → Extensions sets the default for new chats.'}
          </div>
        )}
        <div className="max-h-[400px] overflow-y-auto">
          {sortedExtensions.length === 0 ? (
            <div className="px-3 py-4 text-center text-secondary text-text-muted">
              {searchQuery ? 'No extensions found' : 'No extensions available'}
            </div>
          ) : (
            sortedExtensions.map((ext) => {
              // §14.5, and the reason it is a *state* rather than an omission:
              // dropping the row is what produces "the OMOP tool is broken".
              // Gate C is invisible in the GUI by construction — it returns
              // `ErrorData` from inside `dispatch_tool_call`, so it never
              // enters `PermissionCheckResult`, produces no approval card and
              // records no denial. Nothing else in this app will ever tell the
              // user why the tool did nothing.
              //
              // The row stays in the list and carries `aria-disabled="true"` —
              // Radix derives that attribute from `disabled` below, which is
              // why the string appears nowhere else in this file. Filtering the
              // row out instead would satisfy every other assertion here and
              // reintroduce the exact silence this state removes.
              const pairingRefused = extensionPairingRefused(ext.name, pairingTier);
              const rowDisabled = bulkInFlight || pairingRefused;
              return (
                <DropdownMenuCheckboxItem
                  key={ext.name}
                  checked={ext.enabled}
                  showIndicator={false}
                  disabled={rowDisabled}
                  onCheckedChange={(checked) => handleToggle(ext, checked)}
                  onSelect={(event) => event.preventDefault()}
                  // Geometry comes from the shared §4.5 row (32px, 12px padding,
                  // --radius-md, 13/18, --background-medium hover) — this call site
                  // adds only the toggle layout and the in-flight cursor.
                  className={`justify-between hover:bg-background-medium ${
                    pairingRefused
                      ? 'h-auto cursor-not-allowed items-start py-1.5 opacity-70'
                      : rowDisabled
                        ? 'cursor-wait opacity-70'
                        : 'cursor-pointer'
                  }`}
                  title={pairingRefused ? PAIRING_REFUSED_REASON : ext.description || ext.name}
                >
                  <div className="flex min-w-0 flex-col gap-0.5 pr-2">
                    <div className="flex items-center gap-1.5 min-w-0">
                      <div className="font-medium text-text-default truncate">
                        {formatExtensionName(ext.name)}
                      </div>
                      {isBuiltInExtension(ext) && <BuiltInBadge />}
                    </div>
                    {pairingRefused && (
                      <div className="text-supporting text-text-muted">
                        Unavailable in this chat (public model)
                      </div>
                    )}
                  </div>
                  <div className="pointer-events-none" aria-hidden="true">
                    <Switch
                      checked={ext.enabled}
                      variant="mono"
                      disabled={rowDisabled}
                      tabIndex={-1}
                      aria-hidden="true"
                    />
                  </div>
                </DropdownMenuCheckboxItem>
              );
            })
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
};
