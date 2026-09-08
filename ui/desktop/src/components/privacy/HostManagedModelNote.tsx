import { cn } from '../../utils';
import { Note } from '../ui/note';
import { isBrowserSurface } from '../../utils/surface';
import { HOST_MANAGED_MODEL_REASON, HOST_MANAGED_MODEL_SHORT } from './hostManagedModelCopy';

/**
 * The inline note that sits beside a provider/model control a browser session
 * cannot use (SD-1). See `hostManagedModelCopy.ts` for the ruling and the words.
 *
 * ⚠ **Renders nothing on the desktop**, so a call site can mount it
 * unconditionally. Every surface carrying it would otherwise repeat the same
 * `isBrowserSurface() && …` guard, and the one that forgot would ship a note
 * telling desktop users their model is fixed.
 *
 * ⚠ **`className` MERGES; it used to replace.** `className ?? '…'` meant every
 * call site had to restate the type, and five of the six then disagreed about
 * the box: bare prose on the Models tab, a bordered note on `--background-muted`
 * in `SwitchModelModal` and `LeadWorkerSettings`, a bordered note on
 * `--background-default` in `ProviderGrid`, and a raw `text-[11px] leading-4`
 * divider in `ModelsBottomBar`. `ConfigSettings` had gone further and
 * hand-copied the paragraph outright — a seventh implementation of the one
 * sentence this component exists to keep in one place. The shape now lives
 * here, and `className` carries layout (`mt-*`, `mb-*`) and nothing else.
 *
 * The WORDS do not vary with the variant. `short` picks between the two copy
 * constants and that choice is a decision recorded at each call site, not a
 * style — see the placement comments in `ModelSettingsButtons`,
 * `ResetProviderSection`, `ProviderGrid` and `ModelsBottomBar`.
 */
export type HostManagedModelNoteVariant = 'note' | 'inset';

export function HostManagedModelNote({
  className,
  short = false,
  variant = 'note',
  testId = 'host-managed-model-note',
}: {
  /** Layout only: `mt-*`, `mb-*`, `min-w-0`. The shape belongs to the variant. */
  className?: string;
  /** Use the one-line form, for a chip or a settings row. */
  short?: boolean;
  /**
   * `note` is the boxed neutral {@link Note} — the default, and the shape
   * `SwitchModelModal` and `LeadWorkerSettings` already had. `inset` is the
   * flush, hairline-separated block `ModelsBottomBar` needs *inside* the
   * dropdown's item list, where a rounded card sitting on a card would be
   * wrong.
   */
  variant?: HostManagedModelNoteVariant;
  /** Overridden by `ConfigSettings`, which mounts one per frozen config key. */
  testId?: string;
}) {
  if (!isBrowserSurface()) return null;

  const text = short ? HOST_MANAGED_MODEL_SHORT : HOST_MANAGED_MODEL_REASON;

  if (variant === 'inset') {
    return (
      <p
        data-testid={testId}
        className={cn(
          'border-b border-border-subtle px-3 py-2 text-supporting text-text-muted',
          className
        )}
      >
        {text}
      </p>
    );
  }

  return (
    <Note tone="neutral" testId={testId} className={className}>
      {text}
    </Note>
  );
}
