import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The type scale, asserted at the token (redesign 2026-10, spec §2.1).
 *
 * The owner asked for "very few" font sizes and weights. The app had thirteen
 * sizes in use; the redesign settles on five (12 · 13 · 14 · 17 · 24) and two
 * visible weights, because the face is Arial and Arial draws 400 and 700 only
 * (500 renders as 400, 600 as 700). This file pins the ROLES: what each type
 * token is allowed to be, and then holds the CALL SITES to them (no `text-sm`, no
 * `text-[11px]`, no stray `tracking-*`, no bare `uppercase`, no `font-bold`).
 *
 * The call-site guard is a RATCHET until the redesign's wave 2. A strict guard
 * added while eight owners were still sweeping their files would have failed on
 * arrival and been switched off, so each file that still carries legacy classes
 * is listed with the count it carried when the guard landed. A listed file may
 * only go down; a file that is not listed must be clean. Wave 2 deletes the
 * list, and the guard is then strict.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const UTILS = readFileSync(join(__dirname, '../utils.ts'), 'utf8');
const CODE = CSS.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '));

const SCALE = [12, 13, 14, 17, 24];

/** Every `--text-<role>` declared in a plain `@theme` block, with its parts. */
function roles(): Map<string, Record<string, string>> {
  const out = new Map<string, Record<string, string>>();
  const re = /(^|\n)@theme\s*\{/g;
  while (re.exec(CODE)) {
    let depth = 1;
    let i = re.lastIndex;
    for (; i < CODE.length && depth > 0; i++) {
      if (CODE[i] === '{') depth++;
      else if (CODE[i] === '}') depth--;
    }
    const body = CODE.slice(re.lastIndex, i - 1);
    for (const d of body.matchAll(/--text-([a-z]+)(--[a-z-]+)?:\s*([^;]+);/g)) {
      const [, role, part, value] = d;
      const entry = out.get(role) ?? {};
      entry[part ? part.slice(2) : 'size'] = value.trim();
      out.set(role, entry);
    }
  }
  return out;
}

const ROLES = roles();
const px = (v: string) => Number(/^(\d+)px$/.exec(v)?.[1]);

/** Follow `var(--text-x…)` to the role it aliases. */
function resolve(role: string, part: string): string {
  let value = ROLES.get(role)?.[part] ?? '';
  for (let i = 0; i < 4 && value.startsWith('var('); i++) {
    const m = /^var\(--text-([a-z]+)(?:--([a-z-]+))?\)$/.exec(value);
    if (!m) break;
    value = ROLES.get(m[1])?.[m[2] ?? 'size'] ?? '';
  }
  return value;
}

describe('the type roles', () => {
  it('declares the roles the redesign names', () => {
    expect([...ROLES.keys()].sort()).toEqual(
      [
        'body',
        'caps',
        'chip',
        'code',
        'heading',
        'label',
        'prose',
        'secondary',
        'subheading',
        'supporting',
        'title',
      ].sort()
    );
  });

  it.each([...ROLES.keys()])('%s renders at one of the five sizes', (role) => {
    expect(SCALE).toContain(px(resolve(role, 'size')));
  });

  it.each([...ROLES.keys()])('%s uses a weight of 400, 500 or 600', (role) => {
    expect(['400', '500', '600']).toContain(resolve(role, 'font-weight'));
  });

  it.each([
    ['supporting', '12px', '16px', '400'],
    ['chip', '12px', '16px', '500'],
    ['caps', '12px', '16px', '500'],
    ['secondary', '13px', '18px', '400'],
    ['code', '13px', '20px', '400'],
    ['body', '14px', '20px', '400'],
    ['label', '14px', '20px', '500'],
    ['prose', '14px', '21px', '400'],
    ['subheading', '17px', '24px', '600'],
    ['title', '24px', '32px', '400'],
  ])('%s is %s / %s at %s', (role, size, lineHeight, weight) => {
    expect(resolve(role, 'size')).toBe(size);
    expect(resolve(role, 'line-height')).toBe(lineHeight);
    expect(resolve(role, 'font-weight')).toBe(weight);
  });

  /** One caps style, 12px so it is not a sixth size, tracked +0.04em. */
  it('keeps tracking only on the caps label and the title', () => {
    const tracked = [...ROLES.entries()]
      .filter(([, parts]) => parts['letter-spacing'])
      .map(([role]) => role)
      .sort();
    expect(tracked).toEqual(['caps', 'title']);
    expect(resolve('caps', 'letter-spacing')).toBe('0.04em');
    expect(resolve('title', 'letter-spacing')).toBe('-0.01em');
  });

  /** The retired role is an alias, not a size of its own, and `display` is gone. */
  it('keeps text-heading as an alias until its last use moves, and has no text-display', () => {
    expect(ROLES.get('heading')?.size).toBe('var(--text-subheading)');
    expect(ROLES.has('display')).toBe(false);
  });

  /** `text-caps` carries its casing; the generated utility cannot. */
  it('folds the uppercase transform into text-caps', () => {
    expect(CODE).toMatch(/\.text-caps\s*\{\s*text-transform:\s*uppercase;\s*\}/);
  });
});

/**
 * tailwind-merge has to be taught every role, or `cn()` reads `text-prose` as a
 * text COLOUR and drops the real colour beside it (typography audit R7). The
 * list lives in `src/utils.ts`; this keeps it in step with the stylesheet.
 */
describe('tailwind-merge knows every role', () => {
  const list = /text:\s*\[([^\]]*)\]/.exec(UTILS)?.[1] ?? '';
  const registered = [...list.matchAll(/'([a-z]+)'/g)].map((m) => m[1]).sort();

  it('registers exactly the declared roles', () => {
    expect(registered).toEqual([...ROLES.keys()].sort());
  });
});

/**
 * ── The call sites ────────────────────────────────────────────────────────────
 *
 * What is counted, per file (comments are stripped first, so a note explaining
 * the rule cannot trip it):
 *  - `.ts` / `.tsx`: a legacy size class (`text-xs|sm|base|lg|xl|2xl…`,
 *    `text-[Npx]`, or the retired `text-heading` / `text-display`), a `tracking-*` class, a bare `uppercase` class (`text-caps`
 *    carries its own casing) and `font-bold` (Arial draws 400 and 700, so the
 *    strong weight is `font-semibold`, which the face renders as bold anyway).
 *  - `.css`: a `font-size` in px (a role is a `--text-*` token) and a
 *    `font-weight` of 700 or heavier.
 *
 * Out of scope, each for a reason: `components/crew/` (its own guard,
 * `crewCss.sourceGuard.test.ts`), the frozen files no workstream may restyle
 * (`components/privacy/**`, `context_management/**` except
 * `SystemNotificationInline.tsx`, `LauncherView`, `Pair`, the unmounted
 * `settings/{dictation,tunnel,sessions}/**`), `onboarding/**` and `ProviderGuard`
 * until their P2 sweep lands, the generated `api/`, and tests.
 */
const SRC = join(__dirname, '..');

const OUT_OF_SCOPE = [
  /^components\/crew\//,
  /^components\/privacy\//,
  /^components\/context_management\/(?!SystemNotificationInline)/,
  /^components\/LauncherView\./,
  /^components\/Pair\./,
  /^components\/settings\/(dictation|tunnel|sessions)\//,
  /^components\/onboarding\//,
  /^components\/ProviderGuard\./,
  /^api\//,
];

const CLASS_RULES: Record<string, RegExp> = {
  'legacy size class':
    /(?<![\w-])text-(?:xs|sm|base|lg|[2-9]?xl|heading|display|\[\d+(?:\.\d+)?px\])(?![\w-])/g,
  'tracking class': /(?<![\w-])tracking-[\w[]/g,
  'bare uppercase': /(?<![\w-])uppercase(?![\w-])/g,
  'font-bold': /(?<![\w-])font-bold(?![\w-])/g,
};

const CSS_RULES: Record<string, RegExp> = {
  'px font-size': /font-size:\s*\d*\.?\d+px/g,
  'weight 700 or heavier': /font-weight:\s*(?:700|bold|800|900)\b/g,
};

/** Comments blanked (newlines kept, so line numbers hold). `//` skips `://` in URLs. */
function withoutComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, ' '))
    .replace(/(^|[^:'"`\\])\/\/[^\n]*/g, '$1');
}

/** Every legacy type form in a source file, as `rule @ line`. */
function legacyTypeHits(file: string, source: string): string[] {
  const rules = file.endsWith('.css') ? CSS_RULES : CLASS_RULES;
  const code = withoutComments(source);
  return Object.entries(rules).flatMap(([name, pattern]) =>
    [...code.matchAll(pattern)].map(
      (m) => `${name} @ line ${code.slice(0, m.index).split('\n').length}`
    )
  );
}

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : walk(path);
    return [path];
  });
}

function guardedFiles(): string[] {
  return walk(SRC)
    .map((path) => relative(SRC, path).split(sep).join('/'))
    .filter((file) => /\.(?:tsx?|css)$/.test(file))
    .filter((file) => !/\.test\.|\.browser\.|\.d\.ts$/.test(file))
    .filter((file) => !OUT_OF_SCOPE.some((pattern) => pattern.test(file)));
}

/**
 * The files that still carry legacy forms, with the count each carried when the
 * ratchet landed (2026-10-10). A count may fall and must not rise; a file that
 * is not listed must be clean. Do NOT add a file or raise a count: move the file
 * onto the roles (spec 2.1 has the mapping table). Wave 2 deletes this object.
 */
const SWEEP_PENDING: Record<string, number> = {
  'components/BrxtInstallModal.tsx': 32,
  'components/DependencyErrorBanner.tsx': 2,
  'components/DependencySetupModal.tsx': 18,
  'components/ErrorBoundary.tsx': 5,
  'components/GroupedExtensionLoadingToast.tsx': 1,
  'components/InAppTerminalDock.tsx': 1,
  'components/ModelAndProviderContext.tsx': 5,
  'components/ParameterInputModal.tsx': 2,
  'components/UpdateAvailableModal.tsx': 10,
  'components/alerts/AlertBox.tsx': 8,
  'components/alerts/NotificationSurface.tsx': 2,
  'components/bottom_menu/ModelBreakdownTable.tsx': 1,
  'components/conversation/SearchBar.tsx': 3,
  'components/knowledge/KBSelector/KBManagerDialog.tsx': 2,
  'components/knowledge/KBSelector/KBSelectorMenu.tsx': 1,
  'components/knowledge/KBSelector/KbFormatChooser.tsx': 1,
  'components/knowledge/KnowledgeView.tsx': 1,
  'components/knowledge/changelog/ChangeKindChip.tsx': 1,
  'components/knowledge/changelog/ChangeLogDrawer.tsx': 1,
  'components/knowledge/graph/EdgePreview.tsx': 2,
  'components/parameter/ParameterInput.tsx': 9,
  'components/settings/config/ConfigSettings.tsx': 2,
  'components/settings/extensions/ExtensionsSection.tsx': 1,
  'components/settings/extensions/modal/EnvVarsSection.tsx': 4,
  'components/settings/extensions/modal/ExtensionConfigFields.tsx': 4,
  'components/settings/extensions/modal/ExtensionCredentialsDialog.tsx': 1,
  'components/settings/extensions/modal/ExtensionInfoFields.tsx': 4,
  'components/settings/extensions/modal/ExtensionModal.tsx': 8,
  'components/settings/extensions/modal/ExtensionTimeoutField.tsx': 2,
  'components/settings/extensions/modal/HeadersSection.tsx': 4,
  'components/settings/extensions/subcomponents/ExtensionItem.tsx': 5,
  'components/settings/models/subcomponents/LeadWorkerSettings.tsx': 20,
  'components/settings/models/subcomponents/SwitchModelModal.tsx': 15,
  'components/settings/providers/ProviderSettingsPage.tsx': 2,
  'components/settings/providers/modal/CodingAgentSetupRecovery.tsx': 2,
  'components/settings/providers/modal/ProviderConfiguationModal.tsx': 3,
  'components/settings/providers/modal/subcomponents/ProviderSetupActions.tsx': 2,
  'components/settings/providers/modal/subcomponents/SecureStorageNotice.tsx': 1,
  'components/settings/providers/modal/subcomponents/forms/CustomProviderForm.tsx': 14,
  'components/settings/providers/modal/subcomponents/forms/DefaultProviderSetupForm.tsx': 7,
  'components/settings/providers/subcomponents/CardHeader.tsx': 1,
  'components/settings/providers/subcomponents/ProviderCard.tsx': 4,
  'components/settings/providers/subcomponents/utils/StringUtils.tsx': 1,
  'components/settings/usage/UsagePanel.tsx': 2,
  'components/ui/DangerousConfirmDialog.tsx': 2,
  'components/ui/badge.tsx': 3,
  'styles/main.css': 13,
};

describe('the call sites use the type roles', () => {
  const files = guardedFiles();

  it('is looking at the whole renderer', () => {
    expect(files.length).toBeGreaterThan(500);
    expect(files).toContain('components/ui/button.tsx');
    expect(files).toContain('styles/main.css');
    expect(files.some((file) => file.startsWith('components/crew/'))).toBe(false);
  });

  it('lets no file carry more legacy forms than its baseline, and no clean file gain one', () => {
    const over: string[] = [];
    for (const file of files) {
      const hits = legacyTypeHits(file, readFileSync(join(SRC, file), 'utf8'));
      const allowed = SWEEP_PENDING[file] ?? 0;
      if (hits.length > allowed) {
        over.push(`${file}: ${hits.length} (allowed ${allowed}): ${hits.slice(0, 3).join('; ')}`);
      }
    }
    expect(over, 'use a text-<role> from main.css; see implementation-spec 2.1').toEqual([]);
  });

  /** Guards the guard: each form must be seen, and prose about it must not be. */
  it('recognises each legacy form and ignores comments about them', () => {
    const tsx = [
      '<p className="text-sm font-medium">a</p>',
      '<p className="md:text-[11px]">b</p>',
      '<p className="uppercase tracking-wide">c</p>',
      '<p className="font-bold">d</p>',
    ].join('\n');
    expect(legacyTypeHits('a.tsx', tsx)).toEqual([
      'legacy size class @ line 1',
      'legacy size class @ line 2',
      'tracking class @ line 3',
      'bare uppercase @ line 3',
      'font-bold @ line 4',
    ]);
    const prose = [
      '// text-sm is retired, so is tracking-wide and uppercase',
      '/* font-bold and text-[11px] */',
      '{/* uppercase */}',
      '<p className="text-supporting text-caps text-secondary text-text-muted font-semibold">e</p>',
      '<p className="context-lg normal-case">f</p>',
    ].join('\n');
    expect(legacyTypeHits('b.tsx', prose)).toEqual([]);
    const css =
      '.a { font-size: 13px; }\n.b { font-weight: 700; }\n.c { font-size: var(--text-body); }';
    expect(legacyTypeHits('c.css', css)).toEqual([
      'px font-size @ line 1',
      'weight 700 or heavier @ line 2',
    ]);
  });
});
