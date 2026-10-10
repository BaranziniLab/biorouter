/**
 * One answer to "which glyph does this tool call draw", for every surface that
 * draws one: the transcript row (`ToolCallWithResponse`), a Code Execution
 * script's inner calls (`ExecutedCallRow`), the parked approval card
 * (`PendingToolCallCard`), and an extension's own card in Extensions.
 *
 * WHY IT REPLACED `toolIconMapping.tsx` (now a shim over this module). That
 * module switched on the BARE tool name (everything after the last `__`), so:
 *   - every built-in family it did not list drew the wrench: knowledge, Crew,
 *     todo, workspace, skills, the extension manager, Auto Visualiser, DataSQL,
 *     workflows, schedules, code execution. In a normal transcript that is
 *     most rows;
 *   - two extensions' same-named tools collided: `agent_drafter__list_apps`
 *     drew Copilot's Monitor, because `computercontroller__list_apps` exists;
 *   - `text_editor` drew the edit mark for a `view`.
 *
 * THE LADDER, first match wins:
 *   1. the exact full name             (`workspace__subagent`, `platform__manage_schedule`)
 *   2. an argument-aware tool          (`text_editor` view vs write)
 *   3. the extension's family          (`knowledge__*`, `crew__*`, `todo__*`, …)
 *   4. a per-tool table for the families that are a toolbox, not a concept
 *      (`developer`, `webdocuments`, `files`, `platform`), then its default
 *   5. a coding-agent child's own tool (`Bash`, `Read`, `apply_patch`, `exec`, …)
 *   6. any other `extension__tool`     -> the extension glyph (Puzzle)
 *   7. nothing matched                 -> the wrench
 *
 * ⚠ **Rungs 1 to 4 apply only to Biorouter's own extensions**
 * ({@link BUILT_IN_EXTENSION_KEYS}). Every installed or external extension
 * (SPOKEAgent, CDWAgent, PlaywrightAgent, any MCP server from BAAM or the
 * config) draws the one Puzzle, whatever its tools are called, so an external
 * `acme__read_file` is not drawn as a file read (owner message 11: specific
 * glyphs only for the built-in extensions and tools).
 *
 * The family glyphs ARE the entity glyphs (`ENTITY_ICONS`), so a knowledge
 * tool row, the Knowledge nav row and the composer's knowledge count can never
 * disagree (design.md §3.9, "one glyph, one meaning").
 */
import type { ComponentType } from 'react';
import {
  Archive,
  Bookmark,
  Bug,
  ChartColumn,
  CheckCircle2,
  Code,
  Copilot,
  Database,
  FilePen,
  FileSpreadsheet,
  FileText,
  FolderOpen,
  Globe,
  History,
  Image,
  ListTodo,
  MessagesSquare,
  Search,
  Terminal,
  Wrench,
} from '../components/icons/app-icons';
import { ENTITY_ICONS } from '../components/icons/entity-icons';

export type ToolIcon = ComponentType<{ className?: string }>;

export type ToolGlyphKind =
  // what the call DID (toolbox families and coding-agent child tools)
  | 'shell'
  | 'read'
  | 'write'
  | 'list'
  | 'search'
  | 'web'
  | 'image'
  | 'document'
  | 'spreadsheet'
  | 'archive'
  | 'code'
  // which Biorouter component it USED (entity glyphs)
  | 'knowledge'
  | 'crew'
  | 'workflow'
  | 'schedule'
  | 'skill'
  | 'extension'
  | 'app'
  | 'agent'
  // other built-in components
  | 'workspace'
  | 'figure'
  | 'copilot'
  | 'database'
  | 'memory'
  | 'plan'
  | 'recall'
  | 'bug'
  | 'done'
  // nothing known
  | 'unknown';

export const TOOL_GLYPHS: Record<ToolGlyphKind, ToolIcon> = {
  shell: Terminal,
  read: FileText,
  write: FilePen,
  list: FolderOpen,
  search: Search,
  web: Globe,
  image: Image,
  // A document tool reads or writes one: the same page as a read, on purpose.
  document: FileText,
  spreadsheet: FileSpreadsheet,
  archive: Archive,
  code: Code,
  knowledge: ENTITY_ICONS.knowledge,
  crew: ENTITY_ICONS.crew,
  workflow: ENTITY_ICONS.workflow,
  schedule: ENTITY_ICONS.schedule,
  skill: ENTITY_ICONS.skill,
  extension: ENTITY_ICONS.extension,
  app: ENTITY_ICONS.application,
  agent: ENTITY_ICONS.agent,
  workspace: MessagesSquare,
  figure: ChartColumn,
  copilot: Copilot,
  database: Database,
  memory: Bookmark,
  plan: ListTodo,
  recall: History,
  bug: Bug,
  done: CheckCircle2,
  unknown: Wrench,
};

/**
 * The extension keys Biorouter itself ships: the bundled MCP servers
 * (`config/extensions.rs` `BUILTIN_EXTENSION_METADATA`), the platform
 * extensions (`agents/extension.rs` `PLATFORM_EXTENSIONS`), the in-process
 * servers an app session injects (`routes/apps.rs` `add_inprocess_server`),
 * and the two tool prefixes that are not registered extensions (`platform`,
 * `workflow`). A key is `name_to_key` of the display name: whitespace
 * stripped, lowercased. `toolGlyph.test.ts` checks this set against the crates.
 */
export const BUILT_IN_EXTENSION_KEYS: ReadonlySet<string> = new Set([
  'developer',
  'knowledge',
  'memory',
  'webdocuments',
  'autovisualiser',
  'computercontroller',
  'agent_drafter',
  'appcontrol',
  'datasql',
  'files',
  'compute',
  'evidence',
  'todo',
  'workspace',
  'skills',
  'extensionmanager',
  'code_execution',
  'chatrecall',
  'crew',
  'platform',
  'workflow',
]);

/** `name_to_key` (`config/extensions.rs`): whitespace stripped, lowercased. */
function nameToKey(name: string): string {
  return name.replace(/\s+/g, '').toLowerCase();
}

/**
 * Display names that do not reduce to their key, so an Extensions card that
 * only knows its title still finds its glyph. The labels come from
 * `BUILTIN_EXTENSION_METADATA` and `bundled_extension_display_name`, and
 * `toolGlyph.test.ts` reads both to keep this table complete.
 */
const DISPLAY_KEY_ALIASES: Record<string, string> = Object.fromEntries(
  (
    [
      ['Biorouter Copilot', 'computercontroller'],
      ['Web & Documents', 'webdocuments'],
      ['Agent Drafter', 'agent_drafter'],
      ['Workspace Control', 'workspace'],
      ['Code Execution', 'code_execution'],
    ] as const
  ).map(([label, key]) => [nameToKey(label), key])
);

function extensionKey(nameOrKey: string): string {
  const key = nameToKey(nameOrKey);
  return DISPLAY_KEY_ALIASES[key] ?? key;
}

/** Is this one of Biorouter's own extensions (by key or display name)? */
export function isBuiltInExtension(nameOrKey: string): boolean {
  return BUILT_IN_EXTENSION_KEYS.has(extensionKey(nameOrKey));
}

/**
 * The prefix a coding-agent bridge puts on Biorouter's own tools
 * (`coding_agent/mirror.rs`, `BRIDGE_TOOL_PREFIX`). The daemon already strips
 * it for display; stripping it here too keeps an older stored message correct.
 */
const BRIDGE_PREFIX = 'mcp__biorouter__';

export interface ParsedToolName {
  /** The extension key (`developer`, `knowledge`, …), or '' for a bare name. */
  extension: string;
  /** The tool's own name (`shell`, `kb_search`, `Bash`, …). */
  tool: string;
}

/**
 * `developer__shell` -> developer / shell; `mcp__biorouter__developer__shell`
 * -> the same; `mcp__github__create_issue` (a child agent's own MCP server)
 * -> github / create_issue; `exec` -> '' / exec.
 *
 * Splits at the FIRST `__`: extension keys are `name_to_key` output
 * (whitespace stripped, lowercased) and never contain `__`, while a tool name
 * occasionally does.
 */
export function parseToolName(name: string): ParsedToolName {
  let rest = name.startsWith(BRIDGE_PREFIX) ? name.slice(BRIDGE_PREFIX.length) : name;
  if (rest.startsWith('mcp__')) rest = rest.slice('mcp__'.length);
  const at = rest.indexOf('__');
  return at <= 0
    ? { extension: '', tool: rest }
    : { extension: rest.slice(0, at), tool: rest.slice(at + 2) };
}

/** 1. Exact full names: where a family default would say the wrong thing. */
const EXACT: Record<string, ToolGlyphKind> = {
  workspace__subagent: 'agent',
  workflow__final_output: 'done',
  platform__manage_schedule: 'schedule',
  platform__manage_workflow: 'workflow',
  platform__ingest_conversation: 'knowledge',
  platform__ingest_source: 'knowledge',
  platform__read_session_blob: 'read',
  platform__report_bug: 'bug',
  code_execution__read_module: 'read',
  code_execution__search_modules: 'search',
  agent_drafter__consult: 'agent',
};

/** 3. Built-in families that ARE one Biorouter concept: every tool draws it. */
const FAMILY: Record<string, ToolGlyphKind> = {
  knowledge: 'knowledge',
  crew: 'crew',
  skills: 'skill',
  extensionmanager: 'extension',
  todo: 'plan',
  workspace: 'workspace',
  autovisualiser: 'figure',
  computercontroller: 'copilot',
  datasql: 'database',
  memory: 'memory',
  chatrecall: 'recall',
  agent_drafter: 'app',
  appcontrol: 'app',
  evidence: 'app',
  code_execution: 'code',
  compute: 'code',
  workflow: 'workflow',
};

/** 4. Built-in toolbox families: the action decides, then the family's default. */
const TOOLBOX: Record<string, { tools: Record<string, ToolGlyphKind>; fallback: ToolGlyphKind }> = {
  developer: {
    tools: {
      shell: 'shell',
      shell_status: 'shell',
      shell_kill: 'shell',
      analyze: 'search',
      image_processor: 'image',
    },
    fallback: 'code',
  },
  webdocuments: {
    tools: {
      web_scrape: 'web',
      pdf_tool: 'document',
      docx_tool: 'document',
      xlsx_tool: 'spreadsheet',
      cache: 'archive',
    },
    fallback: 'web',
  },
  files: {
    tools: { files_list: 'list', files_read: 'read', files_write: 'write' },
    fallback: 'read',
  },
  // Every platform tool is named in EXACT; a new one draws the Puzzle until
  // it is, and the drift guard in toolGlyph.test.ts fails on it.
  platform: { tools: {}, fallback: 'extension' },
};

/** 5. Tools a coding-agent child runs itself (Claude Code, Codex) and the
 * legacy bare names `toolIconMapping.tsx` knew. Matched only when the name
 * carries NO extension prefix, so `acme__Read` stays the acme extension's. */
const BARE: Record<string, ToolGlyphKind> = {
  Bash: 'shell',
  BashOutput: 'shell',
  KillShell: 'shell',
  exec: 'shell',
  exec_command: 'shell',
  shell: 'shell',
  Read: 'read',
  NotebookRead: 'read',
  read: 'read',
  Write: 'write',
  Edit: 'write',
  MultiEdit: 'write',
  NotebookEdit: 'write',
  apply_patch: 'write',
  create_file: 'write',
  update_file: 'write',
  Grep: 'search',
  search: 'search',
  Glob: 'list',
  LS: 'list',
  WebFetch: 'web',
  WebSearch: 'web',
  web_search: 'web',
  browser_tabs: 'web',
  view_image: 'image',
  Task: 'agent',
  subagent: 'agent',
  TodoWrite: 'plan',
  update_plan: 'plan',
  sheets_tool: 'spreadsheet',
  docs_tool: 'document',
  final_output: 'done',
};

/** `text_editor`'s command decides read versus write; no command (a pending
 * call whose arguments have not streamed yet) reads as an edit, which is what
 * the tool is for. */
function textEditorKind(args?: Record<string, unknown> | null): ToolGlyphKind {
  return args?.command === 'view' ? 'read' : 'write';
}

export interface ToolGlyph {
  kind: ToolGlyphKind;
  Icon: ToolIcon;
}

export function toolGlyphKind(name: string, args?: Record<string, unknown> | null): ToolGlyphKind {
  const { extension, tool } = parseToolName(name);

  if (extension) {
    // 6. An external extension is the Puzzle, whatever its tools are called.
    if (!BUILT_IN_EXTENSION_KEYS.has(extension)) return 'extension';

    const exact = EXACT[`${extension}__${tool}`];
    if (exact) return exact;
    if (extension === 'developer' && tool === 'text_editor') return textEditorKind(args);
    const family = FAMILY[extension];
    if (family) return family;
    const toolbox = TOOLBOX[extension];
    if (toolbox) return toolbox.tools[tool] ?? toolbox.fallback;
    return 'extension';
  }

  if (tool === 'text_editor') return textEditorKind(args);
  return BARE[tool] ?? 'unknown';
}

export function toolGlyphFor(name: string, args?: Record<string, unknown> | null): ToolGlyph {
  const kind = toolGlyphKind(name, args);
  return { kind, Icon: TOOL_GLYPHS[kind] };
}

/**
 * The glyph an extension's own card draws in Extensions, from the same tables
 * its tool rows use, so the card and the rows agree. Takes the key or the
 * display name. A built-in toolbox draws its default action; every external
 * extension draws the Puzzle.
 */
export function extensionGlyphFor(nameOrKey: string): ToolGlyph {
  const key = extensionKey(nameOrKey);
  const kind = BUILT_IN_EXTENSION_KEYS.has(key)
    ? (FAMILY[key] ?? TOOLBOX[key]?.fallback ?? 'extension')
    : 'extension';
  return { kind, Icon: TOOL_GLYPHS[kind] };
}
