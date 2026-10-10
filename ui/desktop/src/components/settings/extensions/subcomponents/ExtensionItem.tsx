import { useState, useEffect, useId } from 'react';
import kebabCase from 'lodash/kebabCase';
import { Switch } from '../../../ui/switch';
import { Badge } from '../../../ui/badge';
import { InfoTip } from '../../../ui/info-tip';
import { RowActions, RowContextMenu, type RowActionItem } from '../../../ui/row-actions';
import { ENTITY_ICONS } from '../../../icons/entity-icons';
import { FixedExtensionEntry } from '../../../ConfigContext';
import BuiltInBadge from '../../../ui/BuiltInBadge';
import { PrivacyBadge } from '../../../ui/PrivacyBadge';
import { extensionGlyphFor } from '../../../../utils/toolGlyph';
import { getSubtitle, getFriendlyTitle, isBuiltInExtension } from './ExtensionList';
import { classifyExtension, extensionPairingRefused } from '../extensionPrivacy';
import type { DefaultProvider } from '../ExtensionsSection';
import { extensionProvenance, type RegistryLoad } from '../../../baam/registry';
import { EXTENSION_ROW_COPY } from '../copy';
import './extension-row.css';

interface ExtensionItemProps {
  extension: FixedExtensionEntry;
  onToggle: (extension: FixedExtensionEntry) => Promise<boolean | void> | void;
  onConfigure?: (extension: FixedExtensionEntry) => void;
  /** Open the configure dialog already asking to remove. Omitted where nothing can be removed. */
  onRemove?: (extension: FixedExtensionEntry) => void;
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

/**
 * One extension: Crew's row (spec 3.11).
 *
 * A glyph, the name with its badges, and ONE muted line under it. A `⋯` menu (Configure…,
 * Remove…) appears when the row is hovered or focused, and the same menu opens on right-click,
 * Shift+F10 and the ContextMenu key. The switch is named by the row's title, the same in both
 * states: the state is the switch's `aria-checked`, not part of its name (spec 2.6).
 *
 * What used to sit under the title, up to four lines, and where it went:
 *   - the command line: it is in the Configure dialog, which is where it is edited;
 *   - the built-in sentence: the Privacy badge's InfoTip;
 *   - the pairing notice: a warning badge in the title line, its sentence in that badge's InfoTip;
 *   - the §13.5 provenance sentences: STAY, on the same muted line as the description. They are
 *     privacy disclosures (spec decision 6.7), so they are text, not help.
 */
export default function ExtensionItem({
  extension,
  onToggle,
  onConfigure,
  onRemove,
  isStatic,
  defaultProvider,
  catalog,
}: ExtensionItemProps) {
  const [visuallyEnabled, setVisuallyEnabled] = useState(extension.enabled);
  const [isToggling, setIsToggling] = useState(false);
  const titleId = useId();
  const secondaryId = useId();

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

  const { description } = getSubtitle(extension);
  const title = getFriendlyTitle(extension);

  const editable =
    !isStatic && (extension.type === 'builtin' || !('bundled' in extension && extension.bundled));
  // Only an extension the user added can be removed; a built-in is turned off, never deleted.
  const removable = editable && extension.type !== 'builtin' && Boolean(onRemove);

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
  const pairingRefused = Boolean(
    defaultProvider && extensionPairingRefused(extension.name, defaultProvider.tier)
  );

  /**
   * §13.5's strings. **How** an extension got here is the thing a user cannot
   * otherwise see, and it is what decides the badge: an extension installed from
   * a file is Public under R11(ii) because the install records no provenance at
   * all, and nothing on this card said so.
   *
   * A built-in gets its own sentence rather than one of the two marketplace
   * ones, which would both be false statements: it is not published on BAAM and
   * it was not installed from a file. That sentence is not one of the three the
   * design quotes, so it lives in the badge's InfoTip; the three stay visible.
   *
   * The marketplace ones wait for the catalogue to load, because "published" is
   * a claim only the catalogue can back.
   */
  const builtIn = isBuiltInExtension(extension);
  const provenance = builtIn
    ? null
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

  // One glyph per component: the same answer its tool rows give in a chat. Only Biorouter's own
  // extensions get a specific glyph (owner message 11); every external one draws the Puzzle.
  // Gated on the row's own `builtIn` flag as well, so an external server a user happened to name
  // "memory" or "files" still draws the Puzzle.
  const Glyph = builtIn ? extensionGlyphFor(extension.name).Icon : ENTITY_ICONS.extension;

  const menu: RowActionItem[] = editable
    ? [
        { label: EXTENSION_ROW_COPY.configure, onSelect: () => onConfigure?.(extension) },
        ...(removable
          ? ([
              { kind: 'separator' },
              {
                label: EXTENSION_ROW_COPY.remove,
                onSelect: () => onRemove?.(extension),
                destructive: true,
              },
            ] satisfies RowActionItem[])
          : []),
      ]
    : [];

  const privacyBadge = <PrivacyBadge tier={tier} />;

  return (
    <RowContextMenu items={menu} disabled={menu.length === 0}>
      <div
        id={`extension-${kebabCase(extension.name)}`}
        className="biorouter-list-row flex items-center gap-3 px-3 py-2.5"
        data-extension-row=""
      >
        <Glyph className="br-ext-glyph" aria-hidden="true" />
        <div className="br-ext-text">
          {/*
           * ⚠ `min-w-0` belongs on the TITLE, not only on the column above it. A
           * flex item's `min-width` is `auto`, so a title refuses to shrink below
           * its own min-content and pushes the badges — which carry
           * `flex-shrink-0` from `Badge` itself, so they cannot give way — out
           * past the row's trailing switch. `.br-ext-title` is authored CSS for
           * that reason, and the full name is the Configure dialog's.
           */}
          <div className="br-ext-title-line">
            <p id={titleId} className="br-ext-title text-label text-text-default">
              {title}
            </p>
            {builtIn && <BuiltInBadge />}
            {builtIn ? (
              // A built-in's provenance is the badge's help. The span is the focusable trigger
              // (PrivacyBadge takes no props to forward), and the sentence is also kept in a
              // visually hidden node, so it is still in the page for a screen reader.
              <InfoTip
                label={EXTENSION_ROW_COPY.privacy}
                help={EXTENSION_ROW_COPY.builtInProvenance}
                asChild
              >
                <span className="br-ext-badge-target">{privacyBadge}</span>
              </InfoTip>
            ) : (
              privacyBadge
            )}
            {pairingRefused && defaultProvider && (
              // A privacy STATE, so it stays visible; the sentence names the provider the card
              // judged against and is the badge's help and hidden text.
              <InfoTip
                label={EXTENSION_ROW_COPY.unavailable}
                help={EXTENSION_ROW_COPY.unavailableHelp(extension.enabled, defaultProvider.name)}
                asChild
              >
                <Badge tone="warning">{EXTENSION_ROW_COPY.unavailable}</Badge>
              </InfoTip>
            )}
          </div>
          {(description || provenance) && (
            <p id={secondaryId} className="br-ext-secondary text-supporting text-text-muted">
              {description && <span className="br-ext-description">{description}</span>}
              {provenance && <span className="br-ext-provenance">{provenance}</span>}
            </p>
          )}
        </div>
        {menu.length > 0 && <RowActions menu={menu} />}
        {/* Named by the row's title, the same in both states. */}
        <Switch
          checked={isToggling ? visuallyEnabled : extension.enabled}
          onCheckedChange={() => handleToggle(extension)}
          disabled={isToggling}
          aria-labelledby={titleId}
          aria-describedby={description || provenance ? secondaryId : undefined}
        />
      </div>
    </RowContextMenu>
  );
}
