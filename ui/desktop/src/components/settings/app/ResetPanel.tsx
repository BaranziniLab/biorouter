import { useCallback, useId, useState } from 'react';
import { previewReset, resetAppData } from '../../../api';
import type { ResetCategory, ResetCounts } from '../../../api';
import { toastService } from '../../../toasts';
import { clearAllSessionCache } from '../../../utils/sessionCache';
import { clearSessionListCache } from '../../../utils/sessionListCache';
import { LocalMessageStorage } from '../../../utils/localMessageStorage';
import { userActionHeaders } from '../../../utils/userAction';
import { Button } from '../../ui/button';
import { Checkbox } from '../../ui/Checkbox';
import { InfoTip } from '../../ui/info-tip';
import { Note } from '../../ui/note';
import { SettingRow, SettingSection } from '../../ui/setting-row';
import { ModalShell } from '../../ModalShell';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { resetBrowserReason } from './resetOnBrowser';
import { resetCopy } from './copy';

type ResetPanelProps = {
  onReset?: (categories: ResetCategory[]) => void;
};

type CategoryDefinition = {
  id: ResetCategory;
  title: string;
  description: string;
  countKey: keyof ResetCounts;
  countLabel: string;
  singularCountLabel?: string;
};

export const RESET_CATEGORIES: CategoryDefinition[] = [
  {
    id: 'applications',
    title: 'Built apps',
    description: 'Delete every app created with Agent Drafter.',
    countKey: 'applications',
    countLabel: 'built',
  },
  {
    id: 'knowledge',
    title: 'Knowledge & memory',
    description: 'Recreate the empty Soul knowledge base and clear saved memory.',
    countKey: 'knowledgeBases',
    countLabel: 'bases',
    singularCountLabel: 'base',
  },
  {
    id: 'skills',
    title: 'Installed skills',
    description: 'Remove user-installed skills and restore built-in skill files.',
    countKey: 'skills',
    countLabel: 'custom',
  },
  {
    id: 'extensions',
    title: 'Custom extensions',
    description: 'Remove added extensions while keeping bundled capabilities.',
    countKey: 'extensions',
    countLabel: 'custom',
  },
  {
    id: 'schedules',
    title: 'Schedules',
    description: 'Remove custom schedules and restore Daily Meditation.',
    countKey: 'schedules',
    countLabel: 'custom',
  },
  {
    id: 'workflows',
    title: 'Workflows',
    description: 'Remove managed workflows and restore the Meditation workflow.',
    countKey: 'workflows',
    countLabel: 'custom',
  },
  {
    id: 'history',
    title: 'Chat & usage history',
    description: 'Clear every chat, token meter, cost total and checkpoint.',
    countKey: 'conversations',
    countLabel: 'chats',
    singularCountLabel: 'chat',
  },
];

const ALL_CATEGORIES = RESET_CATEGORIES.map((category) => category.id);

// Dashboard mode is gone, but installs that ran an older build still carry its
// localStorage payload. Reset stays responsible for clearing it so the keys do
// not linger forever; drop this once those versions are out of circulation.
const DISCONTINUED_DASHBOARD_KEYS = [
  'biorouter.dashboard.v2',
  'biorouter.dashboard.v1',
  'biorouter.labmeeting.v1',
];

function clearRendererHistory() {
  LocalMessageStorage.clearHistory();
  clearAllSessionCache();
  clearSessionListCache();
  for (const key of DISCONTINUED_DASHBOARD_KEYS) {
    localStorage.removeItem(key);
  }
}

function clearKnowledgeSelections() {
  for (let index = localStorage.length - 1; index >= 0; index -= 1) {
    const key = localStorage.key(index);
    if (key?.startsWith('knowledge_active_kb') || key?.startsWith('knowledge_hidden_kbs')) {
      localStorage.removeItem(key);
    }
  }
}

/**
 * The sentence a failed reset call carries.
 *
 * Under `throwOnError` the generated client throws the PARSED BODY, not an
 * `Error` (`api/client/client.gen.ts`). This route's failures are
 * `ResetErrorResponse` objects, so reading `Error.message` alone replaced every
 * one of them (the 403 saying why a reset was refused, the 409 saying a chat is
 * still running) with the generic fallback.
 */
function resetErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message) return error.message;
  if (typeof error === 'string' && error.trim()) return error;
  if (error && typeof error === 'object' && 'message' in error) {
    const { message } = error as { message: unknown };
    if (typeof message === 'string' && message.trim()) return message;
  }
  return fallback;
}

function countText(category: CategoryDefinition, counts: ResetCounts | null): string | null {
  const count = counts?.[category.countKey];
  if (count === undefined) return null;
  const noun =
    count === 1 && category.singularCountLabel ? category.singularCountLabel : category.countLabel;
  return `${count.toLocaleString()} ${noun}`;
}

/**
 * Settings > App > Danger zone (spec §3.13): ONE row, "Reset data", whose `Reset…` button opens
 * the dialog that holds the whole decision: the seven categories as checkboxes (each one's
 * meaning in an InfoTip), Select all, the permanence line, and the one destructive button,
 * which reads "Reset everything" when every box is checked.
 *
 * Crew's pattern (`crew/pane/AboutTab.tsx`): a page never shows the checklist or a red button
 * beside a paragraph; the consequence is stated in the dialog, at the moment of decision.
 */
export default function ResetPanel({ onReset }: ResetPanelProps) {
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<Set<ResetCategory>>(new Set());
  const [counts, setCounts] = useState<ResetCounts | null>(null);
  const [resetting, setResetting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [completed, setCompleted] = useState(false);
  const idPrefix = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  /**
   * SD-8: on a browser-served page the daemon refuses every reset, so the row says so in place
   * of a working control instead of letting the person select, confirm and then read a refusal.
   * See `resetOnBrowser.ts`. Read from the DOM marker on every render, as `isBrowserSurface`
   * requires.
   */
  const hostOnly = resetBrowserReason();

  const loadCounts = useCallback(async () => {
    // The preview is refused on that surface for the same reason as the reset.
    if (resetBrowserReason()) {
      setCounts(null);
      return;
    }
    try {
      // ⚠ The proof on BOTH calls. The daemon answers the preview and the reset
      // only for a request that proves the person at the keyboard sent it: the
      // counts include every private chat and base, and a reset deletes them.
      const response = await previewReset<true>({
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      setCounts(response.data.counts);
    } catch (loadError) {
      console.error('Failed to inspect reset data:', loadError);
      setCounts(null);
    }
  }, []);

  const openDialog = () => {
    if (resetBrowserReason()) return;
    setSelected(new Set());
    setError(null);
    setCompleted(false);
    setOpen(true);
    void loadCounts();
  };

  const toggleCategory = (category: ResetCategory) => {
    setError(null);
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(category)) next.delete(category);
      else next.add(category);
      return next;
    });
  };

  const selectedCategories = RESET_CATEGORIES.filter((category) => selected.has(category.id)).map(
    (category) => category.id
  );
  const everything = selectedCategories.length === ALL_CATEGORIES.length;

  const handleReset = async () => {
    if (selectedCategories.length === 0 || resetBrowserReason()) return;
    const categories = selectedCategories;
    setResetting(true);
    setError(null);
    try {
      await resetAppData<true>({
        body: { categories },
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      if (categories.includes('history')) clearRendererHistory();
      if (categories.includes('knowledge')) clearKnowledgeSelections();
      setSelected(new Set());
      setOpen(false);
      setCompleted(true);
      onReset?.(categories);
      window.dispatchEvent(new CustomEvent('biorouter:data-reset', { detail: { categories } }));
      toastService.success({
        title: resetCopy.completeToastTitle,
        msg: resetCopy.completeToast(categories.length),
      });
    } catch (resetError) {
      console.error('Failed to reset app data:', resetError);
      const message = resetErrorMessage(resetError, resetCopy.failedFallback);
      // The refusal is shown in the dialog that caused it, and as a toast.
      setError(message);
      toastService.error({ title: resetCopy.failedToastTitle, msg: message });
    } finally {
      setResetting(false);
    }
  };

  return (
    <SettingSection
      id={SETTINGS_SECTION_IDS.appDangerZone}
      title={resetCopy.section}
      data-testid="reset-panel"
    >
      <SettingRow
        label={resetCopy.row}
        help={resetCopy.rowHelp}
        status={completed ? resetCopy.complete : undefined}
      >
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={hostOnly !== null}
          onClick={openDialog}
        >
          {resetCopy.open}
        </Button>
      </SettingRow>

      {hostOnly !== null && (
        // The whole reason, beside the control it disables, so the person reads
        // it before reaching for the control rather than after.
        <Note tone="neutral" testId="reset-needs-host-note" className="mt-2">
          {hostOnly}
        </Note>
      )}

      <ModalShell
        open={open}
        onOpenChange={(next) => !next && !resetting && setOpen(false)}
        size="md"
        purpose={resetting ? 'required' : 'form'}
        title={resetCopy.dialogTitle}
        subtitle={resetCopy.rowHelp}
        footer={
          <>
            <Button
              type="button"
              variant="outline"
              disabled={resetting}
              onClick={() => setOpen(false)}
            >
              {resetCopy.cancel}
            </Button>
            <Button
              type="button"
              variant="destructive"
              disabled={resetting || selectedCategories.length === 0}
              onClick={handleReset}
            >
              {resetting
                ? resetCopy.resetting
                : everything
                  ? resetCopy.resetEverything
                  : resetCopy.resetSelected}
            </Button>
          </>
        }
      >
        <div className="flex items-center justify-between gap-2 pb-2">
          <span className="text-supporting tabular-nums text-text-muted">
            {resetCopy.selectedCount(selectedCategories.length, ALL_CATEGORIES.length)}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={resetting}
            onClick={() => setSelected(everything ? new Set() : new Set(ALL_CATEGORIES))}
          >
            {everything ? resetCopy.clear : resetCopy.selectAll}
          </Button>
        </div>

        <ul className="biorouter-settings-list">
          {RESET_CATEGORIES.map((category) => {
            const checkboxId = `${idPrefix}-reset-${category.id}`;
            const count = countText(category, counts);
            return (
              <li
                key={category.id}
                data-testid={`reset-option-${category.id}`}
                className="biorouter-settings-row flex min-w-0 items-center gap-2 px-3 py-2.5"
              >
                <Checkbox
                  id={checkboxId}
                  checked={selected.has(category.id)}
                  disabled={resetting}
                  onChange={() => toggleCategory(category.id)}
                />
                <label
                  htmlFor={checkboxId}
                  className="min-w-0 truncate text-label text-text-default"
                >
                  {category.title}
                </label>
                <InfoTip label={category.title} help={category.description} />
                {count && (
                  <span className="ml-auto shrink-0 text-supporting tabular-nums text-text-muted">
                    {count}
                  </span>
                )}
              </li>
            );
          })}
        </ul>

        {/* Essential, so it stays visible at the moment of decision (principle 2). */}
        <p className="pt-3 text-supporting text-text-muted">{resetCopy.permanence}</p>

        {error && (
          <Note tone="danger" role="alert" className="mt-3">
            {error}
          </Note>
        )}
      </ModalShell>
    </SettingSection>
  );
}
