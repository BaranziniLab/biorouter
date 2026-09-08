import { useState, useEffect } from 'react';
import kebabCase from 'lodash/kebabCase';
import { Switch } from '../../../ui/switch';
import { Button } from '../../../ui/button';
import { Settings } from '../../../icons/app-icons';
import { FixedExtensionEntry } from '../../../ConfigContext';
import BuiltInBadge from '../../../ui/BuiltInBadge';
import { PrivacyBadge } from '../../../ui/PrivacyBadge';
import { getSubtitle, getFriendlyTitle, isBuiltInExtension } from './ExtensionList';
import { classifyExtension, extensionPairingRefused } from '../extensionPrivacy';
import type { DefaultProvider } from '../ExtensionsSection';
import { extensionProvenance, type RegistryLoad } from '../../../baam/registry';

interface ExtensionItemProps {
  extension: FixedExtensionEntry;
  onToggle: (extension: FixedExtensionEntry) => Promise<boolean | void> | void;
  onConfigure?: (extension: FixedExtensionEntry) => void;
  isStatic?: boolean;
  /**
   * The global default provider (issue #56, §14.5) — the only tier this screen
   * can honestly judge against, since Settings has no session.
   */
  defaultProvider?: DefaultProvider | null;
  /**
   * The marketplace catalogue (issue #56, §13.5), resolved once by
   * `ExtensionsSection`. `null` until it has loaded — the card simply says less
   * until then, rather than guessing a provenance and correcting itself.
   */
  catalog?: RegistryLoad | null;
}

export default function ExtensionItem({
  extension,
  onToggle,
  onConfigure,
  isStatic,
  defaultProvider,
  catalog,
}: ExtensionItemProps) {
  const [visuallyEnabled, setVisuallyEnabled] = useState(extension.enabled);
  const [isToggling, setIsToggling] = useState(false);

  const handleToggle = async (ext: FixedExtensionEntry) => {
    if (isToggling) return;
    setIsToggling(true);
    const newState = !ext.enabled;
    setVisuallyEnabled(newState);
    try {
      await onToggle(ext);
    } catch {
      setVisuallyEnabled(!newState);
    } finally {
      setIsToggling(false);
    }
  };

  useEffect(() => {
    if (!isToggling) {
      setVisuallyEnabled(extension.enabled);
    }
  }, [extension.enabled, isToggling]);

  const { description, command } = getSubtitle(extension);

  const editable =
    !isStatic && (extension.type === 'builtin' || !('bundled' in extension && extension.bundled));

  /**
   * §14.5's third state. A static badge describes a property of the extension;
   * what the user needs is a property of the *pairing*, because `config.yaml`
   * enables extensions globally and there is no per-session enablement — so
   * "Enabled" here and "absent from every chat" are both true at once, and only
   * one of them is on screen.
   *
   * The scope is stated out loud. This card cannot see a chat, so it judges the
   * default provider and names it; the composer's selector answers the
   * per-chat question.
   */
  const pairingNotice =
    defaultProvider && extensionPairingRefused(extension.name, defaultProvider.tier)
      ? `${extension.enabled ? 'Enabled, but u' : 'U'}navailable in new chats (default model is public). Judged against your default provider, ${defaultProvider.name}`
      : null;

  /**
   * §13.5's strings. **How** an extension got here is the thing a user cannot
   * otherwise see, and it is what decides the badge: an extension installed from
   * a file is Public under R11(ii) because the install records no provenance at
   * all, and nothing on this card said so.
   *
   * A built-in gets its own sentence rather than one of the two marketplace
   * ones, which would both be false statements: it is not published on BAAM and
   * it was not installed from a file. Saying nothing was the previous answer and
   * left the row that most obviously ships with the app as the only row that
   * would not say where it came from — and `BuiltInBadge` beside the title says
   * *that it is built in*, not what any model may do with it, which is the half
   * §13.5 exists to state.
   *
   * The built-in sentence needs no catalogue, so it renders immediately; the two
   * marketplace ones wait for the load, because "published" is a claim only the
   * catalogue can back.
   */
  const builtIn = isBuiltInExtension(extension);
  const provenance = builtIn
    ? 'Public: built into Biorouter, not on the marketplace. Any model can call it.'
    : catalog
      ? extensionProvenance(catalog.registry, extension.name)
      : null;

  /**
   * §13.5's other half: *a badge* plus provenance, on every card. The sentence
   * is the explanation; the pill is what survives being skimmed, and a user
   * scanning twenty rows for the one that is Private reads pills.
   *
   * `classifyExtension`, deliberately the same call `extensionPairingRefused`
   * above makes, so one card can never show a Public badge beside an
   * "unavailable in new chats" notice that only a Private extension earns. It
   * also needs no catalogue, so the badge does not appear a beat after the row.
   */
  const tier = classifyExtension(extension.name);

  return (
    <div
      id={`extension-${kebabCase(extension.name)}`}
      className="biorouter-list-row flex items-center gap-4 px-3 py-3 group"
    >
      <div className="flex-1 min-w-0">
        {/*
         * ⚠ `min-w-0` belongs on the TITLE, not only on the column above it. A
         * flex item's `min-width` is `auto`, so this <p> refuses to shrink below
         * its own min-content and pushes the two badges — which carry
         * `flex-shrink-0` from `Badge` itself, so they cannot give way — out
         * past the row's trailing switch. `break-words` would not help: it
         * breaks lines, it does not lower min-content.
         *
         * The case that made it visible is this view moving onto the 760px chat
         * measure, where the content box is ~704px: a long marketplace name
         * beside a `Private (enforcement off)` pill overruns it. `min-w-0
         * truncate` is the same construction `schedule/SchedulesView.tsx` uses
         * on its own row title, and `title` keeps the full name reachable.
         */}
        <div className="flex items-center gap-1.5">
          <p
            className="min-w-0 truncate text-sm font-medium text-text-default leading-snug"
            title={getFriendlyTitle(extension)}
          >
            {getFriendlyTitle(extension)}
          </p>
          {builtIn && <BuiltInBadge />}
          <PrivacyBadge tier={tier} />
        </div>
        {description && (
          <p className="text-xs text-text-muted mt-0.5 line-clamp-1">{description}</p>
        )}
        {command && <p className="text-xs font-mono text-text-muted mt-0.5 truncate">{command}</p>}
        {provenance && <p className="text-xs text-text-subtle mt-1">{provenance}</p>}
        {/* The only line here that interpolates a value from config — the
            default provider's display name — so it is the only one that can
            meet a token with no break opportunity in it and no ceiling to clip
            it. The other four are a truncated title, a clamped description, a
            truncated command and one of four fixed sentences. */}
        {pairingNotice && (
          <p className="text-xs text-text-muted mt-1 [overflow-wrap:anywhere]">{pairingNotice}</p>
        )}
      </div>
      <div className="flex items-center gap-2 flex-shrink-0">
        {editable && (
          /*
           * ⚠ This was a raw <button> with no box at all — no height, width,
           * padding, radius or hit target — and its reveal rule was
           * `opacity-0 group-hover:opacity-100` with neither an `sm:` guard nor
           * `group-focus-within`, unlike the other four row-action clusters.
           *
           * That is an accessibility defect, not a style one. The control stays
           * focusable while invisible, so keyboard users tab to something they
           * cannot see; and below the `sm` breakpoint, where there is no hover,
           * it is unreachable entirely. The reveal string now matches its
           * siblings, so focus reveals it and small screens keep it visible.
           */
          <Button
            variant="ghost"
            shape="round"
            className="opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100"
            aria-label={`Configure ${getFriendlyTitle(extension)} extension`}
            onClick={() => onConfigure?.(extension)}
          >
            <Settings className="w-4 h-4" />
          </Button>
        )}
        <Switch
          checked={isToggling ? visuallyEnabled : extension.enabled}
          onCheckedChange={() => handleToggle(extension)}
          disabled={isToggling}
          variant="mono"
          aria-label={`Toggle ${getFriendlyTitle(extension)} extension`}
        />
      </div>
    </div>
  );
}
