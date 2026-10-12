import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import { ItemIcon } from './ItemIcon';
import BuiltInBadge from './ui/BuiltInBadge';
import { DROPDOWN_ROW_CLASS_NAME as MENU_ROW_CLASS_NAME } from './ui/dropdown-menu';
import { Spinner } from './ui/spinner';
import { COMPOSER_COPY } from './composer/copy';
import { mentionMenuPlacement } from './composer/mentionMenuPlacement';
import { cn } from '../utils';
import './composer/composer.css';
import { CommandType, getSessionExtensions, getSlashCommands, listBases } from '../api';
import { userActionHeaders } from '../utils/userAction';
import type { CatalogView } from '../api';
import { readKnowledgeSelection, type KnowledgeSelection } from './knowledge/knowledgeSelection';
import { getInitialWorkingDir } from '../utils/workingDir';
import { IMAGE_EXTENSIONS } from '../utils/imageFormats';
import { labelledRefTag, refTag, type RefKind } from '../utils/resourceRefs';
import { isChipCommand, joinLeadingCommand } from '../utils/composerCommand';
import { useConfig } from './ConfigContext';
import { fetchSkillCatalog, pickerBundles, standaloneSkills } from './skills/useSkillCatalog';
import { extensionReferenceItems } from './extensionReferenceItems';

type DisplayItemType = CommandType | 'Directory' | 'File' | 'KnowledgeBase' | 'Skill' | 'Extension';

const typeOrder: Record<DisplayItemType, number> = {
  Builtin: 0,
  Workflow: 1,
  KnowledgeBase: 2,
  Skill: 3,
  Extension: 4,
  Directory: 5,
  File: 6,
};

// Slash commands that are purely a UI convenience. Keyed by command name (no
// leading '/'). `reference` routes the command through the same reference path
// as a picked resource, so the composer draws it as a chip instead of leaving a
// second visual language for the same thing; `insert` is for commands that are
// not a resource at all.
const CLIENT_INSERT_COMMANDS: Record<
  string,
  { description: string; insert?: string; reference?: MentionReference }
> = {
  knowledge: {
    description: COMPOSER_COPY.commands.knowledge,
    reference: { kind: 'extension', value: 'knowledge', label: undefined },
  },
  crew: {
    description: COMPOSER_COPY.commands.crew,
    insert: '/crew',
  },
  diverge: {
    description: COMPOSER_COPY.commands.diverge,
    insert: '/diverge',
  },
};

const REMOVED_SLASH_COMMANDS = new Set(['prompt', 'prompts']);

/**
 * `/crew` in a chat with no session yet (UXN-8): Enter can only say to send a message first, so the
 * row says so before it is picked, and stays where people look for Crew.
 */
export const CREW_COMMAND_BEFORE_FIRST_MESSAGE = COMPOSER_COPY.commands.crewBeforeFirstMessage;

// ── Crew in the palette (live QA round 2, Q2-71; round 3, Q3-31) ───────────────────────────────
// "/cr" listed two rows that both read as "crew": the `/crew` command, which connects the chat, and
// the Crew extension's reference, described as "Use saved Crew connections and human-approved
// channel context…". People asked "which crew?". The command is the one a person means: it ranks
// first for any query it starts with. Round 3 still found the second row confusing, so the
// extension's row is left out of any list that shows the `/crew` command. It stays reachable: a
// query aimed at extensions ("/ext", "/ext:crew") never matches the command, so that list shows
// the extension's row — labelled as the tools, for someone who knows they want them.

/** The Crew extension's row, when a query reaches it without the `/crew` command. */
export const CREW_EXTENSION_DESCRIPTION = COMPOSER_COPY.commands.crewExtension;

/** The Crew platform extension's key (as `isCrewExtensionName` in `crew/access` reads it). */
const isCrewExtensionItem = (item: DisplayItem) =>
  item.itemType === 'Extension' && item.relativePath.trim().toLowerCase() === 'crew';

const withoutSlash = (value: string) => value.replace(/^\/+/, '').toLowerCase();

/** The client's `/crew` command (Q2-10): connects this chat to a Crew channel. */
const isCrewCommandItem = (item: DisplayItem) =>
  item.itemType === 'Builtin' && withoutSlash(item.name) === 'crew';

/** A list showing the `/crew` command, without the Crew extension's row beside it (Q3-31). */
const withoutCrewExtensionBesideCommand = <T extends DisplayItem>(items: T[]): T[] =>
  items.some(isCrewCommandItem) ? items.filter((item) => !isCrewExtensionItem(item)) : items;

/** A built-in command whose name the query starts: it outranks every fuzzy match. */
const isBuiltinPrefixMatch = (item: DisplayItem, query: string) => {
  const typed = withoutSlash(query.trim());
  return (
    item.itemType === 'Builtin' && typed.length > 0 && withoutSlash(item.name).startsWith(typed)
  );
};
/** The resource a picked item refers to, for the composer's chip rail. */
export interface MentionReference {
  kind: RefKind;
  /** The identity the agent resolves: a skill/extension name, or a KB id. */
  value: string;
  /** The display string, where it differs from the identity. */
  label: string | undefined;
}

const REFERENCE_KIND: Partial<Record<DisplayItemType, RefKind>> = {
  KnowledgeBase: 'knowledge_base',
  Skill: 'skill',
  Extension: 'extension',
};

/**
 * What a knowledge-base row may say about the chat, given what the daemon said.
 *
 * With no selection the read failed, and a failure answers neither question the
 * other two labels claim to — is this base in the chat, is it the primary — so
 * the row names only what it is. It is still offered: a reference names its
 * base by id, and an explicit id reaches a base whatever the chat's selection
 * (`kb_id_or_primary` in the knowledge server).
 */
const knowledgeBaseRole = (selection: KnowledgeSelection | null, kbId: string) => {
  if (!selection) return 'Knowledge base';
  return selection.primaryKbId === kbId ? 'Primary knowledge base' : 'Knowledge base in this chat';
};

/**
 * The resource `item` names, or `null` if it is not a resource reference.
 *
 * ## Why this is a tag and no longer a compact marker (issue #65)
 *
 * The backend extracted `/skill:` `/ext:` `/kb:` with `extract_inline_refs`,
 * which splits the message on whitespace — so `/skill:my skill` reached the
 * resolver as `my` and matched nothing, and the agent reported the resource
 * unknown, which reads exactly like a policy refusal.
 *
 * Issue #60 fixed that for `/ext:` by dropping the whitespace, which was
 * lossless *there*: an extension's canonical key is the name with whitespace
 * removed and lowercased, and both `/ext:` consumers already discard it. That
 * fix cannot transfer to skills. `SkillsClient::handle_load_skill` looks a skill
 * up with an exact `self.skills.get(name)` on a map keyed by the frontmatter
 * name, so `myskill` is not `my skill` either — normalising would have traded a
 * visible truncation for a quieter failure.
 *
 * `<biorouter-ref …>` delimits the value explicitly instead of by whitespace,
 * so a name survives it whatever it contains. Nothing is collapsed any more:
 * `Chat Recall` is inserted as `Chat Recall`, and `_`/`-` — which survive the
 * backend's `normalize` and so are part of a user extension's key — keep being
 * part of the name they were configured with.
 */
export const mentionReference = (item: DisplayItem): MentionReference | null => {
  const clientCommand = item.itemType === 'Builtin' ? CLIENT_INSERT_COMMANDS[item.name] : undefined;
  if (clientCommand) return clientCommand.reference ?? null;

  const kind = REFERENCE_KIND[item.itemType];
  if (!kind) return null;

  // A knowledge base is picked by name and resolved by id — `relativePath` is
  // the id `kb_search` takes and `name` carries the `kb:`-prefixed display
  // string the user actually chose. The other kinds are named by the same
  // string they are resolved by, so a label would only be a duplicate.
  const label =
    kind === 'knowledge_base'
      ? item.name.replace(/^kb:/, '')
      : kind === 'extension'
        ? item.name.replace(/^ext:/, '')
        : undefined;

  return { kind, value: item.relativePath, label: label === item.relativePath ? undefined : label };
};

export const getMentionInsertText = (item: DisplayItem) => {
  const reference = mentionReference(item);
  if (reference) {
    return reference.label
      ? labelledRefTag(reference.kind, reference.value, reference.label)
      : refTag(reference.kind, reference.value);
  }

  const clientInsert =
    item.itemType === 'Builtin' ? CLIENT_INSERT_COMMANDS[item.name]?.insert : undefined;
  if (clientInsert) return clientInsert;

  // `/bug ` WITH its space: the composer draws exactly that as a chip, and a
  // bare `/bug` stays text (`utils/composerCommand.ts`). The row itself is the
  // daemon's, so it keeps the daemon's description and the built-in badge.
  const commandName = item.name.replace(/^\/+/, '');
  if (item.itemType === 'Builtin' && isChipCommand(commandName)) {
    return joinLeadingCommand(commandName, '');
  }

  return ['Builtin', 'Workflow'].includes(item.itemType) ? `/${item.name}` : item.extra;
};

export interface DisplayItem {
  name: string;
  extra: string;
  itemType: DisplayItemType;
  relativePath: string;
  builtIn?: boolean;
  searchTerms?: string[];
}

export interface DisplayItemWithMatch extends DisplayItem {
  matchScore: number;
  matches: number[];
  matchedText: string;
}

const uniqueDisplayItems = (displayItems: DisplayItem[]) => {
  const seen = new Set<string>();
  return displayItems.filter((item) => {
    const key = `${item.itemType}\0${item.relativePath}\0${item.name}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
};

/** A path that lists as something with entries is a folder (the way a symlinked one reads). */
const readsAsFolder = async (fullPath: string) => {
  try {
    return (await window.electron.listFiles(fullPath)).length > 0;
  } catch {
    return false;
  }
};

/** A file or a folder found by scanning the chat's working directory. */
const isFileSystemItem = (item: DisplayItem) =>
  item.itemType === 'File' || item.itemType === 'Directory';

/**
 * What a row says under its name. A file or folder says which folder it sits in,
 * relative to the chat's folder, and nothing at all when it sits directly in it
 * (names, not paths: principle 10). The full path is the row's tooltip, and it is
 * still what `extra` holds, because that is what a pick inserts. Any other row
 * says what it is: a command's help, a skill's description.
 */
export const mentionRowDetail = (item: DisplayItem): string | undefined => {
  if (!isFileSystemItem(item)) return item.extra || undefined;
  const slash = item.relativePath.lastIndexOf('/');
  return slash > 0 ? item.relativePath.slice(0, slash) : undefined;
};

const MAX_FILE_SCAN_RESULTS = 300;
const MAX_FILE_DISPLAY_RESULTS = 80;

interface MentionPopoverProps {
  isOpen: boolean;
  onClose: () => void;
  onSelect: (filePath: string) => void;
  /** Where the list opens from: `mentionMenuAnchor` (the text's left edge, the card's top). */
  position: { x: number; y: number };
  query: string;
  isSlashCommand: boolean;
  selectedIndex: number;
  onSelectedIndexChange: (index: number) => void;
  workingDir?: string;
  sessionId?: string | null;
}

// Enhanced fuzzy matching algorithm
const fuzzyMatch = (pattern: string, text: string): { score: number; matches: number[] } => {
  if (!pattern) return { score: 0, matches: [] };

  const patternLower = pattern.toLowerCase();
  const textLower = text.toLowerCase();
  const matches: number[] = [];

  let patternIndex = 0;
  let score = 0;
  let consecutiveMatches = 0;

  for (let i = 0; i < textLower.length && patternIndex < patternLower.length; i++) {
    if (textLower[i] === patternLower[patternIndex]) {
      matches.push(i);
      patternIndex++;
      consecutiveMatches++;

      // Bonus for consecutive matches
      score += consecutiveMatches * 3;

      // Bonus for matches at word boundaries or path separators
      if (
        i === 0 ||
        textLower[i - 1] === '/' ||
        textLower[i - 1] === '_' ||
        textLower[i - 1] === '-' ||
        textLower[i - 1] === '.'
      ) {
        score += 10;
      }

      // Bonus for matching the start of the filename (after last /)
      const lastSlash = textLower.lastIndexOf('/', i);
      if (lastSlash !== -1 && i === lastSlash + 1) {
        score += 15;
      }
    } else {
      consecutiveMatches = 0;
    }
  }

  // Only return a score if all pattern characters were matched
  if (patternIndex === patternLower.length) {
    // Less penalty for longer strings to allow nested files to rank well
    score -= text.length * 0.05;

    // Bonus for exact substring matches
    if (textLower.includes(patternLower)) {
      score += 20;
    }

    // Bonus for matching the filename specifically (not just the path)
    const fileName = text.split('/').pop()?.toLowerCase() || '';
    if (fileName.includes(patternLower)) {
      score += 25;
    }

    return { score, matches };
  }

  return { score: -1, matches: [] };
};

const MentionPopover = forwardRef<
  { getDisplayFiles: () => DisplayItemWithMatch[]; selectFile: (index: number) => void },
  MentionPopoverProps
>(
  (
    {
      isOpen,
      onClose,
      onSelect,
      position,
      query,
      isSlashCommand,
      selectedIndex,
      onSelectedIndexChange,
      workingDir,
      sessionId,
    },
    ref
  ) => {
    const [items, setItems] = useState<DisplayItem[]>([]);
    const [isLoading, setIsLoading] = useState(false);
    // Whether a load has answered since the menu opened as this kind of list. Until
    // one has, an empty list is "Searching…", never "No files in this folder".
    const [hasAnswered, setHasAnswered] = useState(false);
    // Opening, closing, or turning from `/` to `@` starts from an empty list, reset
    // during render so a frame never shows the other list's rows or a stale answer.
    const listKind = !isOpen ? 'closed' : isSlashCommand ? 'commands' : 'files';
    const [loadedKind, setLoadedKind] = useState(listKind);
    if (loadedKind !== listKind) {
      setLoadedKind(listKind);
      setItems([]);
      setIsLoading(false);
      setHasAnswered(false);
    }
    const popoverRef = useRef<HTMLDivElement>(null);
    const listRef = useRef<HTMLDivElement>(null);
    const currentWorkingDir = workingDir ?? getInitialWorkingDir();
    const { extensionsList } = useConfig();

    const scanDirectoryFromRoot = useCallback(
      async (
        dirPath: string,
        relativePath = '',
        depth = 0,
        limitRef = { count: 0 }
      ): Promise<DisplayItem[]> => {
        if (depth > 1 || limitRef.count >= MAX_FILE_SCAN_RESULTS) return [];

        try {
          const items = await window.electron.listFiles(dirPath);
          // Which of these entries are folders. `listFiles` cannot say: it answers an
          // empty list for a file and for an empty folder alike, so asking it about
          // each entry called every file with an unfamiliar extension (a .zip) a
          // folder. One call here names the folders of this listing (V2-10).
          const folderNames = new Set(await window.electron.listSkillDirs(dirPath));
          const results: DisplayItem[] = [];

          // Common directories to prioritize or skip
          const priorityDirs = [
            'Desktop',
            'Documents',
            'Downloads',
            'Projects',
            'Development',
            'Code',
            'src',
            'components',
            'icons',
          ];
          const skipDirs = [
            '.git',
            '.svn',
            '.hg',
            'node_modules',
            '__pycache__',
            'target',
            'dist',
            'build',
            '.cache',
            '.npm',
            '.yarn',
            'Library',
            'System',
            'Applications',
            '.Trash',
          ];

          const allowedHiddenDirs = [
            '.github',
            '.vscode',
            '.idea',
            '.config',
            '.gitlab',
            '.circleci',
            '.azure',
            '.jenkins',
          ];

          const skipDirsAtDepth =
            depth > 1 ? ['.git', '.svn', '.hg', 'node_modules', '__pycache__'] : skipDirs;

          // Sort items to prioritize certain directories
          const sortedItems = items.sort((a, b) => {
            const aPriority = priorityDirs.includes(a);
            const bPriority = priorityDirs.includes(b);
            if (aPriority && !bPriority) return -1;
            if (!aPriority && bPriority) return 1;
            return a.localeCompare(b);
          });

          const itemLimit = depth === 0 ? 40 : 25;
          const queryForRecursion = query.trim();

          for (const item of sortedItems.slice(0, itemLimit)) {
            if (limitRef.count >= MAX_FILE_SCAN_RESULTS) break;
            const fullPath = `${dirPath}/${item}`;
            const itemRelativePath = relativePath ? `${relativePath}/${item}` : item;

            // Skip items in the skip list
            if (skipDirsAtDepth.includes(item)) {
              continue;
            }

            // Skip hidden items except for allowed hidden directories
            if (item.startsWith('.') && !allowedHiddenDirs.includes(item)) {
              continue;
            }

            // First, check if this looks like a file based on extension
            const hasExtension = item.includes('.');
            const ext = item.split('.').pop()?.toLowerCase();
            const commonExtensions = [
              // Code items
              'txt',
              'md',
              'js',
              'ts',
              'jsx',
              'tsx',
              'py',
              'java',
              'cpp',
              'c',
              'h',
              'css',
              'html',
              'json',
              'xml',
              'yaml',
              'yml',
              'toml',
              'ini',
              'cfg',
              'sh',
              'bat',
              'ps1',
              'rb',
              'go',
              'rs',
              'php',
              'sql',
              'r',
              'scala',
              'swift',
              'kt',
              'dart',
              'vue',
              'svelte',
              'astro',
              'scss',
              'less',
              // Documentation
              'readme',
              'license',
              'changelog',
              'contributing',
              // Config items
              'gitignore',
              'dockerignore',
              'editorconfig',
              'prettierrc',
              'eslintrc',
              // Images — spread from the panel's own list so a mention can never
              // offer a format the preview panel will refuse to render.
              ...IMAGE_EXTENSIONS,
              // Vector and design items
              'ai',
              'eps',
              'sketch',
              'fig',
              'xd',
              'psd',
              // Other common items
              'pdf',
              'doc',
              'docx',
              'xls',
              'xlsx',
              'ppt',
              'pptx',
            ];

            // If it has a known file extension, treat it as a file
            if (hasExtension && ext && commonExtensions.includes(ext)) {
              results.push({
                extra: fullPath,
                name: item,
                itemType: 'File',
                relativePath: itemRelativePath,
              });
              limitRef.count += 1;
              continue;
            }

            // If it's a known file without extension (README, LICENSE, etc.)
            const knownFiles = [
              'readme',
              'license',
              'changelog',
              'contributing',
              'dockerfile',
              'makefile',
            ];
            if (!hasExtension && knownFiles.includes(item.toLowerCase())) {
              results.push({
                extra: fullPath,
                name: item,
                itemType: 'File',
                relativePath: itemRelativePath,
              });
              limitRef.count += 1;
              continue;
            }

            // Otherwise it is a folder only when this folder's listing says so. A symlink
            // to a folder is not in that list (the entry is the link), but reading it
            // answers its entries, so a non-empty answer still counts. Anything else
            // that is there is a file whose extension is not in the list above.
            const isFolder = folderNames.has(item) || (await readsAsFolder(fullPath));
            if (!isFolder) {
              results.push({
                extra: fullPath,
                name: item,
                itemType: 'File',
                relativePath: itemRelativePath,
              });
              limitRef.count += 1;
              continue;
            }

            results.push({
              name: item,
              extra: fullPath,
              itemType: 'Directory',
              relativePath: itemRelativePath,
            });
            limitRef.count += 1;

            const shouldRecurse =
              depth === 0 &&
              (priorityDirs.includes(item) ||
                (queryForRecursion.length >= 2 &&
                  fuzzyMatch(queryForRecursion, itemRelativePath).score >=
                    Math.max(8, queryForRecursion.length * 4)));

            if (shouldRecurse) {
              const subFiles = await scanDirectoryFromRoot(
                fullPath,
                itemRelativePath,
                depth + 1,
                limitRef
              );
              results.push(...subFiles);
            }
          }

          return results;
        } catch (error) {
          console.error(`Error scanning directory ${dirPath}:`, error);
          return [];
        }
      },
      [query]
    );

    const scanFilesFromRoot = useCallback(async (): Promise<DisplayItem[]> => {
      try {
        let startPath = currentWorkingDir;

        if (!startPath) {
          if (window.electron.platform === 'win32') {
            startPath = 'C:\\Users';
          } else if (window.electron.platform === 'linux') {
            startPath = '/home';
          } else {
            startPath = '/Users'; // Default to macOS
          }
        }

        return await scanDirectoryFromRoot(startPath);
      } catch (error) {
        console.error('Error scanning items from root:', error);
        return [];
      }
    }, [scanDirectoryFromRoot, currentWorkingDir]);

    const loadReferenceItems = useCallback(
      async (includeCommands: boolean) => {
        // The user's proof on the reads below that name a chat or its knowledge
        // bases: since issue #56's QA sweep (2026-09-10) a request without it is
        // answered as a public model, and would be shown no private base and no
        // private chat's extensions. The selection read carries it itself.
        const headers = await userActionHeaders();
        const [commandsResponse, basesResponse, selection, skillsResult, sessionExtensions] =
          await Promise.all([
            includeCommands
              ? getSlashCommands({ throwOnError: true })
              : Promise.resolve({ data: { commands: [] } }),
            listBases({ headers, throwOnError: false }),
            // Issue #56 Task 58: sent with the user's proof, which a GET naming
            // a PRIVATE chat needs. `null` when the read failed anyway, and
            // `knowledgeBaseRole` then claims nothing about the chat.
            readKnowledgeSelection(sessionId),
            // The daemon's catalog, not a renderer scan: a skill bundled inside
            // an installed extension was loadable by the model and absent from
            // this list, so `@skill:word` completed to nothing (#113).
            fetchSkillCatalog(sessionId).catch(
              () => ({ generation: 0, roots: [], skills: [], bundles: [] }) as CatalogView
            ),
            sessionId
              ? getSessionExtensions({ path: { session_id: sessionId }, headers }).catch(() => null)
              : Promise.resolve(null),
          ]);
        const commandItems: DisplayItem[] = (commandsResponse.data?.commands || [])
          .filter(
            (cmd) => !REMOVED_SLASH_COMMANDS.has(cmd.command.replace(/^\/+/, '').toLowerCase())
          )
          .map((cmd) => ({
            name: cmd.command,
            extra: cmd.help,
            itemType: cmd.command_type,
            relativePath: cmd.command,
            builtIn: cmd.command_type === 'Builtin',
          }));
        if (includeCommands) {
          const existingNames = new Set(commandItems.map((c) => c.name));
          for (const [name, def] of Object.entries(CLIENT_INSERT_COMMANDS)) {
            if (existingNames.has(name) || (name === 'diverge' && !sessionId)) continue;
            commandItems.push({
              name,
              extra:
                name === 'crew' && !sessionId ? CREW_COMMAND_BEFORE_FIRST_MESSAGE : def.description,
              itemType: 'Builtin',
              relativePath: name,
              builtIn: true,
            });
          }
        }

        for (const base of basesResponse.data ?? []) {
          if (selection?.hiddenKbIds.has(base.id)) continue;
          commandItems.push({
            name: `kb:${base.name}`,
            extra: `${knowledgeBaseRole(selection, base.id)} · ${base.id}`,
            itemType: 'KnowledgeBase',
            relativePath: base.id,
          });
        }
        for (const bundle of pickerBundles(skillsResult)) {
          commandItems.push({
            name: `skill:${bundle.name}`,
            extra: `${bundle.skills.length} skill${bundle.skills.length === 1 ? '' : 's'} in bundle`,
            itemType: 'Skill',
            relativePath: bundle.name,
          });
        }
        for (const skill of standaloneSkills(skillsResult)) {
          commandItems.push({
            name: `skill:${skill.name}`,
            extra: skill.description,
            itemType: 'Skill',
            relativePath: skill.name,
          });
        }

        commandItems.push(
          ...extensionReferenceItems(
            sessionId
              ? (sessionExtensions?.data?.extensions ?? [])
              : extensionsList.filter((extension) => extension.enabled)
          ).map((item) =>
            isCrewExtensionItem(item) ? { ...item, extra: CREW_EXTENSION_DESCRIPTION } : item
          )
        );

        return uniqueDisplayItems(commandItems);
      },
      [extensionsList, sessionId]
    );

    const compareByType = (a: DisplayItemWithMatch, b: DisplayItemWithMatch) => {
      const orderA = typeOrder[a.itemType] ?? Number.MAX_SAFE_INTEGER;
      const orderB = typeOrder[b.itemType] ?? Number.MAX_SAFE_INTEGER;
      return orderA - orderB;
    };

    const displayItems = useMemo((): DisplayItemWithMatch[] => {
      const shown = (list: DisplayItemWithMatch[]) =>
        isSlashCommand ? withoutCrewExtensionBesideCommand(list) : list;
      if (!query.trim()) {
        // Nothing typed yet: every command, or the chat folder's entries (its own
        // first, then the ones a level down), so an `@` alone already offers files.
        const unfiltered = items
          .map((file) => ({
            ...file,
            matchScore: 0,
            matches: [],
            matchedText: file.name,
            depth: currentWorkingDir
              ? file.extra.replace(currentWorkingDir, '').split('/').length - 1
              : 0,
          }))
          .sort((a, b) => {
            if (a.depth !== b.depth) return a.depth - b.depth;
            const typeComparison = compareByType(a, b);
            return typeComparison || a.name.localeCompare(b.name);
          });
        return isSlashCommand ? shown(unfiltered) : unfiltered.slice(0, MAX_FILE_DISPLAY_RESULTS);
      }

      const matchedItems = items
        .map((file) => {
          const matches = [
            { match: fuzzyMatch(query, file.name), text: file.name },
            { match: fuzzyMatch(query, file.relativePath), text: file.relativePath },
            ...(file.searchTerms ?? []).map((text) => ({ match: fuzzyMatch(query, text), text })),
          ];
          if (isSlashCommand) {
            matches.push({ match: fuzzyMatch(query, file.extra), text: file.extra });
          }

          const { match: bestMatch, text: matchedText } = matches.reduce((best, current) =>
            current.match.score > best.match.score ? current : best
          );

          let finalScore = bestMatch.score;
          if (finalScore > 0 && currentWorkingDir) {
            const depth = file.extra.replace(currentWorkingDir, '').split('/').length - 1;
            finalScore += depth <= 1 ? 50 : depth <= 2 ? 30 : depth <= 3 ? 15 : 0;
          }

          return {
            ...file,
            matchScore: finalScore,
            matches: bestMatch.matches,
            matchedText,
          };
        })
        .filter((file) => file.matchScore > 0)
        .sort((a, b) => {
          // A built-in command the query starts ranks first: "/cr" means /crew (Q2-71).
          if (isSlashCommand) {
            const prefixDiff =
              Number(isBuiltinPrefixMatch(b, query)) - Number(isBuiltinPrefixMatch(a, query));
            if (prefixDiff) return prefixDiff;
          }
          // Sort by score first, then prefer items over directories, then alphabetically
          const scoreDiff = b.matchScore - a.matchScore;
          if (Math.abs(scoreDiff) >= 1) return scoreDiff;
          const typeComparison = compareByType(a, b);
          return typeComparison || a.name.localeCompare(b.name);
        });

      return isSlashCommand ? shown(matchedItems) : matchedItems.slice(0, MAX_FILE_DISPLAY_RESULTS);
    }, [items, query, currentWorkingDir, isSlashCommand]);

    // Expose methods to parent component
    useImperativeHandle(
      ref,
      () => ({
        getDisplayFiles: () => displayItems,
        selectFile: (index: number) => {
          const item = displayItems[index];
          if (!item) return;
          onSelect(getMentionInsertText(item));
          onClose();
        },
      }),
      [displayItems, onSelect, onClose]
    );

    useEffect(() => {
      if (!isOpen) return;
      let cancelled = false;
      let timeoutId: ReturnType<typeof setTimeout> | undefined;

      // Only the newest load may answer: an older scan that finishes late must not
      // put its rows back, or turn "Searching…" off under a newer one.
      const answer = (next: DisplayItem[]) => {
        if (cancelled) return;
        setItems(next);
        setIsLoading(false);
        setHasAnswered(true);
      };

      setIsLoading(true);
      if (isSlashCommand) {
        loadReferenceItems(true).then(answer, (error) => {
          console.error('Error loading slash commands:', error);
          answer([]);
        });
      } else {
        // An `@` alone scans too (V2-R3-4): picking "Mention" from the `+` menu, or
        // typing `@`, lists the chat folder at once instead of waiting for two
        // characters. Below two characters the scan does not depend on the query
        // (only a longer one recurses into the folders it matches).
        timeoutId = setTimeout(() => {
          void scanFilesFromRoot().then(answer);
        }, 120);
      }

      return () => {
        cancelled = true;
        if (timeoutId) clearTimeout(timeoutId);
      };
    }, [isOpen, isSlashCommand, loadReferenceItems, query, scanFilesFromRoot]);

    useEffect(() => {
      const handleClickOutside = (event: MouseEvent) => {
        if (popoverRef.current && !popoverRef.current.contains(event.target as Node)) {
          onClose();
        }
      };

      if (isOpen) {
        document.addEventListener('mousedown', handleClickOutside);
      }

      return () => {
        document.removeEventListener('mousedown', handleClickOutside);
      };
    }, [isOpen, onClose]);

    // Scroll selected item into view
    useEffect(() => {
      if (listRef.current && selectedIndex >= 0 && selectedIndex < displayItems.length) {
        const selectedElement = listRef.current.children[selectedIndex] as HTMLElement;
        if (selectedElement) {
          // Instant, not smooth: this fires on every arrow-key press, and
          // animated scrolls queue up and lag behind rapid keyboard navigation.
          selectedElement.scrollIntoView({
            block: 'nearest',
            behavior: 'auto',
          });
        }
      }
    }, [selectedIndex, displayItems.length]);

    const handleItemClick = (index: number) => {
      if (index >= 0 && index < displayItems.length) {
        onSelectedIndexChange(index);
        const displayItem = displayItems[index];
        onSelect(getMentionInsertText(displayItem));
        onClose();
      }
    };

    if (!isOpen) return null;

    // "Searching…" only while there is nothing to show: a rescan behind a typed
    // query keeps the rows it has on screen until the new ones arrive.
    const searching = displayItems.length === 0 && (isLoading || !hasAnswered);

    // Above the composer card, 8px clear of its edge (V2-R5-2), and never up
    // into the window-drag band at the top of a chat.
    const placement = mentionMenuPlacement(position, window.innerWidth);

    // The shared menu surface and row recipe (spec 2.6): 12px radius, 4px
    // padding, 32px rows at 13px, the highlight a neutral fill. The list is
    // driven from the textarea (arrows, Enter, Tab), so a row is highlighted by
    // `data-selected`, not by focus.
    const menu = (
      <div
        ref={popoverRef}
        role="listbox"
        aria-label={
          isSlashCommand ? COMPOSER_COPY.mention.commandsLabel : COMPOSER_COPY.mention.mentionsLabel
        }
        data-testid="mention-popover"
        className="biorouter-popover-surface br-mention-menu fixed flex flex-col overflow-hidden rounded-container bg-background-default p-1"
        style={placement}
      >
        {searching ? (
          <div className="flex min-h-control-md items-center gap-2 px-3 text-secondary text-text-muted">
            <Spinner size={14} />
            <span>{COMPOSER_COPY.mention.searching}</span>
          </div>
        ) : (
          <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto">
            {displayItems.map((item, index) => {
              const detail = mentionRowDetail(item);
              return (
                <div
                  key={`${item.itemType}-${item.relativePath}-${item.name}`}
                  role="option"
                  aria-selected={index === selectedIndex}
                  data-selected={index === selectedIndex ? '' : undefined}
                  title={isFileSystemItem(item) ? item.extra : undefined}
                  onClick={() => handleItemClick(index)}
                  onMouseEnter={() => onSelectedIndexChange(index)}
                  className={cn(MENU_ROW_CLASS_NAME, 'cursor-pointer')}
                >
                  <span className="flex-shrink-0 text-text-muted">
                    <ItemIcon item={item} />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex min-w-0 items-center gap-1.5">
                      <span className="truncate text-secondary text-text-default">{item.name}</span>
                      {item.builtIn && <BuiltInBadge />}
                    </span>
                    {detail && (
                      <span className="block truncate text-supporting text-text-muted">
                        {detail}
                      </span>
                    )}
                  </span>
                </div>
              );
            })}

            {displayItems.length === 0 && (query || !isSlashCommand) && (
              <div className="flex min-h-control-md items-center px-3 text-secondary text-text-muted">
                {query ? COMPOSER_COPY.mention.noMatch(query) : COMPOSER_COPY.mention.emptyFolder}
              </div>
            )}
          </div>
        )}
      </div>
    );

    // An empty command list with nothing typed yet draws nothing rather than an
    // empty box. An `@` alone always answers: its entries, "Searching…", or a
    // line saying the folder has nothing to list.
    if (!searching && displayItems.length === 0 && !query && isSlashCommand) return null;

    return createPortal(menu, document.body);
  }
);

MentionPopover.displayName = 'MentionPopover';

export default MentionPopover;
