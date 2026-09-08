import { useCallback, useEffect, useState } from 'react';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { Play } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { BioRouterApp, listApps } from '../../api';
import { useChatContext } from '../../contexts/ChatContext';

/**
 * The card grid, measured against the reading column this view now sits on.
 *
 * At the chat measure the usable width is 760 − 2×24 (`px-6`) − 8 (`p-1`) =
 * 704px, so `auto-fill` with a 280px floor and a 16px gap seats TWO columns of
 * ~344px (2×280 + 16 = 576 ≤ 704; three would need 872). That is the intended
 * outcome: a card falls out of the row rather than being squeezed under its own
 * floor, and 344px is wider than the 280px these cards were designed to survive.
 *
 * The template is an inline `style` rather than a `grid-cols-[…]` arbitrary
 * class on purpose — the same reason `.biorouter-note-clamp` is authored CSS:
 * under `BIOROUTER_NO_HMR` the renderer ignores every watch path, which is the
 * signal Tailwind's scanner uses to notice new class strings, so a freshly
 * written arbitrary utility can silently fail to generate. A grid that fails to
 * generate does not degrade — it stacks every card in one column.
 */
const GridLayout = ({ children }: { children: React.ReactNode }) => {
  return (
    <div
      className="grid gap-4 p-1"
      style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))' }}
    >
      {children}
    </div>
  );
};

/** Loading is cards that are the shape of cards, not a line of prose in dead space. */
const AppCardSkeleton = () => (
  <div className="flex min-w-0 flex-col rounded-container border border-border-subtle bg-background-default p-4">
    <Skeleton className="h-5 w-40" />
    <Skeleton className="mt-3 h-3 w-full" />
    <Skeleton className="mt-2 h-3 w-2/3" />
    <Skeleton className="mt-4 h-6 w-24" />
    <Skeleton className="mt-4 h-8 w-full" />
  </div>
);

export default function AppsView() {
  const [apps, setApps] = useState<BioRouterApp[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const chatContext = useChatContext();
  const sessionId = chatContext?.chat.sessionId;

  // Load cached apps immediately on mount
  useEffect(() => {
    const loadCachedApps = async () => {
      try {
        const response = await listApps({
          throwOnError: true,
        });
        const cachedApps = response.data?.apps || [];
        setApps(cachedApps);
      } catch (err) {
        console.warn('Failed to load cached apps:', err);
      } finally {
        setLoading(false);
      }
    };

    loadCachedApps();
  }, []);

  // When sessionId becomes available, fetch fresh apps and update cache
  useEffect(() => {
    if (!sessionId) return;

    const refreshApps = async () => {
      try {
        const response = await listApps({
          throwOnError: true,
          query: { session_id: sessionId },
        });
        const freshApps = response.data?.apps || [];
        setApps(freshApps);
        setError(null);
      } catch (err) {
        console.warn('Failed to refresh apps:', err);
        // Don't set error if we already have cached apps
        if (apps.length === 0) {
          setError(err instanceof Error ? err.message : 'Failed to load apps');
        }
      }
    };

    refreshApps();
    // apps.length intentionally not in deps: we want to capture the initial apps.length to check
    // "did we have cached apps when refresh started?" Adding it would cause infinite loop since setApps() changes apps.length
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  const loadApps = useCallback(async () => {
    if (!sessionId) return;

    try {
      setLoading(true);
      const response = await listApps({
        throwOnError: true,
        query: { session_id: sessionId },
      });
      const fetchedApps = response.data?.apps || [];
      setApps(fetchedApps);
      setError(null);
    } catch (err) {
      // Only set error if we don't have apps to show
      if (apps.length === 0) {
        setError(err instanceof Error ? err.message : 'Failed to load apps');
      }
    } finally {
      setLoading(false);
    }
  }, [sessionId, apps.length]);

  const handleLaunchApp = async (app: BioRouterApp) => {
    try {
      await window.electron.launchApp(app);
    } catch (err) {
      console.error('Failed to launch app:', err);
      // App launch errors shouldn't hide the apps list, just log it
    }
  };

  return (
    <MainPanelLayout>
      <div className="flex-1 flex flex-col min-h-0">
        {/* The one page header. This view has no actions of its own — the list
            is whatever the installed extensions advertise, so there is nothing
            here to create, import or refresh by hand. `PageHeader` also brings
            the reading column this view never had: its header and its body were
            raw `px-8` divs, so the cards ran the full width of the window while
            every sibling view stopped at a measure. */}
        <PageHeader
          title="MCP apps"
          description="Apps your installed extensions provide, which can run in standalone windows. Apps you built yourself live under Built apps."
        />

        <ReadableContent size="chat" className="min-h-0 flex-1 overflow-y-auto px-6 pt-6 pb-8">
          {/* The load-bearing half of "only show error-only UI if we have no
              apps to display": a refresh that fails while cards are on screen
              must not replace them with an error. It no longer takes the whole
              view with it either — the error was an early `return` above the
              header, so a failed load removed the page's own title. Rule 4: an
              inline error is a `Note`, and Retry rides its action slot. */}
          {error && apps.length === 0 ? (
            <Note
              tone="danger"
              role="alert"
              action={
                <Button variant="outline" size="sm" onClick={loadApps}>
                  Retry
                </Button>
              }
            >
              Error loading apps: {error}
            </Note>
          ) : loading ? (
            <GridLayout>
              <AppCardSkeleton />
              <AppCardSkeleton />
            </GridLayout>
          ) : apps.length === 0 ? (
            <EmptyState
              icon={ENTITY_ICONS.mcpApp}
              title="No apps available"
              description="Install MCP servers that provide UI resources to see apps here."
            />
          ) : (
            <GridLayout>
              {apps.map((app) => (
                /* `tint-interactive`, NOT `hover:bg-overlay-hover`. The overlay
                   tokens are a background COLOUR, so on an opaque card they
                   REPLACE `--background-default` with 5% ink over the canvas —
                   the card loses its surface on hover instead of darkening,
                   which is the same inversion `main.css` records for a filled
                   control. The tint is a background-IMAGE composited over
                   whatever fill is already there, and it carries press as well
                   as hover. `transition-colors` names `--tint-ink`, so the
                   fade eases rather than snapping. */
                <div
                  key={`${app.uri}-${app.mcpServer}`}
                  className="flex min-w-0 flex-col p-4 border border-border-subtle rounded-container bg-background-default tint-interactive transition-colors hover:border-border-strong"
                >
                  <div className="flex-1 min-w-0 mb-4">
                    {/* `[overflow-wrap:anywhere]`, not `break-words`. An MCP app
                        name is a server-authored string with no guaranteed break
                        opportunity, and `overflow-wrap: break-word` changes only
                        where a line MAY break — it leaves the min-content width
                        intact, so a long name still forces the grid track wider
                        than its own card. `anywhere` is the spelling that
                        shrinks it. */}
                    <h3 className="mb-2 min-w-0 text-label text-text-default [overflow-wrap:anywhere]">
                      {app.name}
                    </h3>
                    {app.description && (
                      <p className="mb-2 min-w-0 text-supporting text-text-muted [overflow-wrap:anywhere]">
                        {app.description}
                      </p>
                    )}
                    {/* V8 — the shared `Badge`, in the 24px `chip` tier: the
                        server name is what the app is FILED UNDER, not a status
                        it is currently in. The hand-rolled span it replaces was
                        a fifth spelling of the same neutral chip. */}
                    {app.mcpServer && (
                      <Badge variant="chip" className="max-w-full" title={app.mcpServer}>
                        <span className="truncate">{app.mcpServer}</span>
                      </Badge>
                    )}
                  </div>
                  {/* V7 — the Button carries variant and NOTHING else. It had
                      `className="flex items-center gap-2 flex-1"`: `gap-2` and
                      `items-center` are already in the cva base, and the bare
                      `flex` FLIPS that base's own `inline-flex` through
                      tailwind-merge — the mechanism that once rendered a row
                      action as a full-width bar.

                      The action row is a one-column `grid`, not a `flex`, so the
                      full-width Launch the card was designed with comes from the
                      row stretching its item rather than from a `flex-1` on the
                      control. Two reasons beyond taste: it leaves the Button
                      with no `className` at all, and `flex-1` matches the
                      `\bflex\b` word boundary the vocabulary guard tests Button
                      tags against — a false positive waiting for the day this
                      directory joins its ROOTS list. */}
                  <div className="grid gap-2">
                    <Button variant="default" onClick={() => handleLaunchApp(app)}>
                      <Play />
                      Launch
                    </Button>
                  </div>
                </div>
              ))}
            </GridLayout>
          )}
        </ReadableContent>
      </div>
    </MainPanelLayout>
  );
}
