import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { ENTITY_ICONS } from '../components/icons/entity-icons';
import {
  BUILT_IN_EXTENSION_KEYS,
  TOOL_GLYPHS,
  extensionGlyphFor,
  isBuiltInExtension,
  parseToolName,
  toolGlyphFor,
  toolGlyphKind,
  type ToolGlyphKind,
} from './toolGlyph';
import { getToolCallIcon } from './toolIconMapping';

/**
 * Every tool a built-in, platform or in-process extension registers, as
 * enumerated from `crates/` on 2026-10-09, with the glyph it must draw. The
 * point of the table is the row count: a family that silently falls back to
 * the wrench is exactly the defect this module replaced.
 */
const REAL_TOOLS: Record<ToolGlyphKind, string[]> = {
  shell: ['developer__shell', 'developer__shell_status', 'developer__shell_kill'],
  read: ['files__files_read', 'platform__read_session_blob', 'code_execution__read_module'],
  write: ['files__files_write'],
  list: ['files__files_list'],
  search: ['developer__analyze', 'code_execution__search_modules'],
  web: ['webdocuments__web_scrape'],
  image: ['developer__image_processor'],
  document: ['webdocuments__pdf_tool', 'webdocuments__docx_tool'],
  spreadsheet: ['webdocuments__xlsx_tool'],
  archive: ['webdocuments__cache'],
  code: ['code_execution__execute_code', 'compute__compute_python', 'compute__compute_run'],
  knowledge: [
    'knowledge__kb_search',
    'knowledge__kb_read_page',
    'knowledge__kb_write_page',
    'knowledge__kb_list_bases',
    'knowledge__kb_get_active',
    'knowledge__kb_set_active',
    'knowledge__kb_lint',
    'knowledge__kb_merge',
    'knowledge__kb_begin_txn',
    'knowledge__kb_commit_txn',
    'platform__ingest_conversation',
    'platform__ingest_source',
  ],
  crew: ['crew__connections', 'crew__request'],
  workflow: ['platform__manage_workflow'],
  schedule: ['platform__manage_schedule'],
  skill: [
    'skills__searchSkills',
    'skills__loadSkill',
    'skills__searchMarketplaceSkills',
    'skills__installMarketplaceSkill',
    'skills__importSkillPackage',
    'skills__removeSkillPackage',
    'skills__setSkillEnabled',
  ],
  extension: [
    'extensionmanager__manage_extensions',
    'extensionmanager__search_available_extensions',
    'extensionmanager__install_extension',
    'extensionmanager__browse_marketplace_extensions',
    'extensionmanager__search_marketplace_extensions',
    'extensionmanager__delete_extension_package',
    'extensionmanager__remove_extension',
    'extensionmanager__list_resources',
    'extensionmanager__read_resource',
  ],
  app: [
    'agent_drafter__create_app',
    'agent_drafter__build_app',
    'agent_drafter__list_apps',
    'agent_drafter__preview_app',
    'agent_drafter__export_app',
    'appcontrol__ui_render',
    'appcontrol__ui_ask',
    'evidence__report_evidence',
  ],
  agent: ['workspace__subagent', 'agent_drafter__consult'],
  workspace: [
    'workspace__workspace_list',
    'workspace__workspace_read_conversation',
    'workspace__workspace_send_prompt',
    'workspace__workspace_set_tools',
    'workspace__workspace_close',
    'workspace__workspace_watch',
    'workspace__workspace_open',
    'workspace__workspace_read_panel',
    'workspace__workspace_capture_panel',
  ],
  figure: [
    'autovisualiser__render_figure',
    'autovisualiser__describe_figure',
    'autovisualiser__render_dashboard',
  ],
  copilot: [
    'computercontroller__list_apps',
    'computercontroller__get_app_state',
    'computercontroller__click',
    'computercontroller__perform_secondary_action',
    'computercontroller__scroll',
    'computercontroller__drag',
    'computercontroller__type_text',
    'computercontroller__press_key',
    'computercontroller__set_value',
    'computercontroller__screen_capture',
  ],
  database: ['datasql__data_query', 'datasql__data_sources', 'datasql__data_table'],
  memory: [
    'memory__remember_memory',
    'memory__retrieve_memories',
    'memory__remove_specific_memory',
    'memory__remove_memory_category',
  ],
  plan: [
    'todo__todo_write',
    'todo__todo_add',
    'todo__todo_expand',
    'todo__todo_update',
    'todo__plan_write',
  ],
  recall: ['chatrecall__chatrecall'],
  bug: ['platform__report_bug'],
  done: ['workflow__final_output'],
  unknown: [],
};

describe('toolGlyphKind: every real tool draws its own glyph', () => {
  for (const [kind, names] of Object.entries(REAL_TOOLS)) {
    for (const name of names) {
      it(`${name} -> ${kind}`, () => {
        expect(toolGlyphKind(name)).toBe(kind);
      });
    }
  }
});

describe('the ladder', () => {
  /**
   * ⚠ The defect the full-name match exists for: `toolIconMapping.tsx` keyed on
   * the bare name, so Agent Drafter's `list_apps` drew Copilot's Monitor.
   */
  it('keeps two extensions with the same tool name apart', () => {
    expect(toolGlyphKind('agent_drafter__list_apps')).toBe('app');
    expect(toolGlyphKind('computercontroller__list_apps')).toBe('copilot');
  });

  it('reads text_editor by its command', () => {
    expect(toolGlyphKind('developer__text_editor', { command: 'view', path: 'a.rs' })).toBe('read');
    for (const command of ['write', 'str_replace', 'insert', 'undo_edit']) {
      expect(toolGlyphKind('developer__text_editor', { command })).toBe('write');
    }
    // A pending call whose arguments have not streamed yet is an edit, not a read.
    expect(toolGlyphKind('developer__text_editor')).toBe('write');
    expect(toolGlyphKind('developer__text_editor', null)).toBe('write');
    // An older stored message that carries the bare name.
    expect(toolGlyphKind('text_editor', { command: 'view' })).toBe('read');
  });

  it('strips the coding-agent bridge prefix', () => {
    expect(parseToolName('mcp__biorouter__developer__shell')).toEqual({
      extension: 'developer',
      tool: 'shell',
    });
    expect(toolGlyphKind('mcp__biorouter__knowledge__kb_search')).toBe('knowledge');
    expect(parseToolName('exec')).toEqual({ extension: '', tool: 'exec' });
    expect(parseToolName('a__b__c')).toEqual({ extension: 'a', tool: 'b__c' });
  });

  it("draws a coding-agent child's own tools by what they do", () => {
    const child: Record<string, ToolGlyphKind> = {
      Bash: 'shell',
      exec: 'shell',
      exec_command: 'shell',
      Read: 'read',
      Edit: 'write',
      Write: 'write',
      MultiEdit: 'write',
      apply_patch: 'write',
      Grep: 'search',
      Glob: 'list',
      WebFetch: 'web',
      WebSearch: 'web',
      web_search: 'web',
      view_image: 'image',
      Task: 'agent',
      TodoWrite: 'plan',
      update_plan: 'plan',
    };
    for (const [name, kind] of Object.entries(child)) expect(toolGlyphKind(name), name).toBe(kind);
  });

  /**
   * A child tool's name is only trusted bare: an extension that happens to
   * name a tool `Read` keeps its own glyph.
   */
  it('never reads an extension tool by a bare child name', () => {
    expect(toolGlyphKind('acme__Read')).toBe('extension');
  });

  it('falls back to the extension glyph, then to the wrench', () => {
    expect(toolGlyphKind('acme__do_something')).toBe('extension');
    expect(toolGlyphKind('mcp__github__create_issue')).toBe('extension');
    expect(toolGlyphKind('developer__brand_new_tool')).toBe('code');
    expect(toolGlyphKind('mystery_tool')).toBe('unknown');
    expect(toolGlyphKind('')).toBe('unknown');
    expect(TOOL_GLYPHS.unknown).not.toBe(TOOL_GLYPHS.extension);
  });
});

/**
 * Owner message 11: specific glyphs only for Biorouter's own extensions and
 * tools. A marketplace agent is an external extension like any other, and its
 * tool rows and its Extensions card draw the one Puzzle, whatever its tools
 * are called.
 */
describe('external extensions draw the Puzzle', () => {
  it('resolves marketplace and invented extensions to the extension glyph', () => {
    for (const name of [
      'spokeagent__query_graph',
      'playwrightagent__browser_navigate',
      'ucsfomopagent__run_sql',
      'cdwagent__query',
      'codegraphagent__search',
      'biroffice__docx_edit',
      'acme__read_file',
      'acme__shell',
      'acme__text_editor',
    ]) {
      expect(toolGlyphKind(name, { command: 'view' }), name).toBe('extension');
      expect(toolGlyphFor(name).Icon, name).toBe(ENTITY_ICONS.extension);
    }
  });

  it("draws an external extension's card as the Puzzle", () => {
    for (const name of ['SPOKEAgent', 'spokeagent', 'PlaywrightAgent', 'Some Third Party']) {
      expect(extensionGlyphFor(name).kind, name).toBe('extension');
      expect(isBuiltInExtension(name), name).toBe(false);
    }
  });
});

describe('one glyph per concept', () => {
  /**
   * The family glyphs ARE the entity glyphs. A knowledge tool row, the
   * Knowledge nav row and the composer's knowledge count draw one figure.
   */
  it('takes every component family from ENTITY_ICONS', () => {
    expect(TOOL_GLYPHS.knowledge).toBe(ENTITY_ICONS.knowledge);
    expect(TOOL_GLYPHS.crew).toBe(ENTITY_ICONS.crew);
    expect(TOOL_GLYPHS.workflow).toBe(ENTITY_ICONS.workflow);
    expect(TOOL_GLYPHS.schedule).toBe(ENTITY_ICONS.schedule);
    expect(TOOL_GLYPHS.skill).toBe(ENTITY_ICONS.skill);
    expect(TOOL_GLYPHS.extension).toBe(ENTITY_ICONS.extension);
    expect(TOOL_GLYPHS.app).toBe(ENTITY_ICONS.application);
    expect(TOOL_GLYPHS.agent).toBe(ENTITY_ICONS.agent);
  });

  /** `read` and `document` share FileText on purpose (a document tool reads or
   * writes one); every other kind is its own figure. */
  it('gives every other kind a distinct figure', () => {
    const SHARED = new Set<ToolGlyphKind>(['document']);
    const icons = (Object.keys(TOOL_GLYPHS) as ToolGlyphKind[])
      .filter((k) => !SHARED.has(k))
      .map((k) => TOOL_GLYPHS[k]);
    expect(new Set(icons).size).toBe(icons.length);
  });

  /** The acceptance transcript: one distinct glyph per family, and the wrench
   * only for the unknown tool. */
  it('draws ten families with ten different glyphs', () => {
    const calls: Array<[string, Record<string, unknown>?]> = [
      ['developer__shell'],
      ['developer__text_editor', { command: 'view' }],
      ['developer__text_editor', { command: 'str_replace' }],
      ['developer__analyze'],
      ['knowledge__kb_search'],
      ['crew__request'],
      ['todo__todo_write'],
      ['autovisualiser__render_figure'],
      ['workspace__workspace_list'],
      ['spokeagent__query_graph'],
    ];
    const icons = calls.map(([name, args]) => toolGlyphFor(name, args).Icon);
    expect(new Set(icons).size).toBe(icons.length);
    expect(icons).not.toContain(TOOL_GLYPHS.unknown);
    expect(toolGlyphFor('mystery_tool').Icon).toBe(TOOL_GLYPHS.unknown);
  });

  it("draws an extension's card with the glyph its tool rows draw", () => {
    for (const key of ['knowledge', 'computercontroller', 'datasql', 'memory', 'autovisualiser']) {
      expect(extensionGlyphFor(key).Icon, key).toBe(TOOL_GLYPHS[toolGlyphKind(`${key}__x`)]);
    }
    // Toolboxes draw their default action.
    expect(extensionGlyphFor('developer').kind).toBe('code');
    expect(extensionGlyphFor('webdocuments').kind).toBe('web');
  });

  it('finds a built-in card by its display name', () => {
    expect(extensionGlyphFor('Biorouter Copilot').kind).toBe('copilot');
    expect(extensionGlyphFor('Web & Documents').kind).toBe('web');
    expect(extensionGlyphFor('Auto Visualiser').kind).toBe('figure');
    expect(extensionGlyphFor('Agent Drafter').kind).toBe('app');
    expect(extensionGlyphFor('Workspace Control').kind).toBe('workspace');
    expect(extensionGlyphFor('Code Execution').kind).toBe('code');
    expect(extensionGlyphFor('Chat Recall').kind).toBe('recall');
    expect(extensionGlyphFor('Extension Manager').kind).toBe('extension');
  });
});

/** The shim keeps the old callers on the new ladder. */
describe('toolIconMapping shim', () => {
  it('answers from toolGlyph', () => {
    expect(getToolCallIcon('knowledge__kb_search')).toBe(ENTITY_ICONS.knowledge);
    expect(getToolCallIcon('developer__text_editor', false, { command: 'view' })).toBe(
      TOOL_GLYPHS.read
    );
    expect(getToolCallIcon('spokeagent__query_graph')).toBe(ENTITY_ICONS.extension);
    expect(getToolCallIcon('memory__remember_memory', true)).toBe(TOOL_GLYPHS.memory);
  });
});

// ---------------------------------------------------------------------------
// Drift guards: read the Rust source, the same direction
// `autovis_cdn_desktop_contract.rs` reads the desktop source from Rust.
// ---------------------------------------------------------------------------

const CRATES = join(__dirname, '../../../../crates');
const read = (path: string) => readFileSync(join(CRATES, path), 'utf8');
const nameToKey = (name: string) => name.replace(/\s+/g, '').toLowerCase();

/**
 * `BUILT_IN_EXTENSION_KEYS` is the boundary between "draws its own glyph" and
 * "draws the Puzzle", so it must be exactly the set of keys Biorouter ships.
 * Collected from every place the crates declare one.
 */
describe('BUILT_IN_EXTENSION_KEYS matches the crates', () => {
  function shippedKeys(): Map<string, string> {
    const found = new Map<string, string>(); // key -> where

    // Bundled MCP servers, with the labels the app shows.
    const meta = read('biorouter/src/config/extensions.rs');
    const table = meta.match(/const BUILTIN_EXTENSION_METADATA[\s\S]*?=\s*&\[([\s\S]*?)\];/)?.[1];
    expect(table, 'BUILTIN_EXTENSION_METADATA not found').toBeTruthy();
    for (const m of table!.matchAll(/\(\s*"([^"]+)",\s*"([^"]+)"/g)) {
      found.set(m[1], 'BUILTIN_EXTENSION_METADATA');
    }

    // Platform extensions: a literal name or a module's EXTENSION_NAME.
    const platform = read('biorouter/src/agents/extension.rs');
    for (const m of platform.matchAll(/PlatformExtensionDef\s*\{\s*name:\s*([^,]+),/g)) {
      const expr = m[1].trim();
      let name = expr.match(/^"([^"]+)"$/)?.[1];
      if (!name) {
        const module = expr.match(/(\w+)::EXTENSION_NAME$/)?.[1];
        expect(module, `unreadable platform name ${expr}`).toBeTruthy();
        const src = read(`biorouter/src/agents/${module}.rs`);
        name = src.match(/EXTENSION_NAME:\s*&(?:'static\s+)?str\s*=\s*"([^"]+)"/)?.[1];
      }
      expect(name, `no name for ${expr}`).toBeTruthy();
      found.set(nameToKey(name!), 'PLATFORM_EXTENSIONS');
    }

    // The `platform__*` prefix (not a registered extension).
    const tools = read('biorouter/src/agents/platform_tools.rs');
    const prefix = tools.match(/PLATFORM_EXTENSION_NAME:\s*&str\s*=\s*"([^"]+)"/)?.[1];
    expect(prefix).toBeTruthy();
    found.set(prefix!, 'platform_tools.rs');

    // The workflow's final-output tool prefix.
    const finalOutput = read('biorouter/src/agents/final_output_tool.rs');
    const workflow = finalOutput.match(/FINAL_OUTPUT_TOOL_NAME:\s*&str\s*=\s*"(\w+?)__/)?.[1];
    expect(workflow).toBeTruthy();
    found.set(workflow!, 'final_output_tool.rs');

    // In-process servers an app session injects.
    const apps = read('biorouter-server/src/routes/apps.rs');
    for (const m of apps.matchAll(/add_inprocess_server\(\s*"([^"]+)"/g)) {
      found.set(m[1], 'routes/apps.rs');
    }

    // Subprocess servers (`biorouter mcp <name>`).
    const runner = read('biorouter-mcp/src/mcp_server_runner.rs');
    for (const m of runner.matchAll(/"(\w+)"\s*=>\s*Ok\(McpCommand::/g)) {
      found.set(m[1], 'mcp_server_runner.rs');
    }
    return found;
  }

  it('lists every key the crates ship, and nothing else', () => {
    const shipped = shippedKeys();
    // The scan must find a real number of keys, or it proves nothing.
    expect(shipped.size).toBeGreaterThanOrEqual(15);
    for (const [key, where] of shipped) {
      expect(BUILT_IN_EXTENSION_KEYS.has(key), `${key} (${where}) is missing`).toBe(true);
    }
    for (const key of BUILT_IN_EXTENSION_KEYS) {
      expect(shipped.has(key), `${key} is not shipped by any crate`).toBe(true);
    }
  });

  it('finds every bundled extension by the label the app shows', () => {
    const meta = read('biorouter/src/config/extensions.rs');
    const table = meta.match(/const BUILTIN_EXTENSION_METADATA[\s\S]*?=\s*&\[([\s\S]*?)\];/)![1];
    for (const m of table.matchAll(/\(\s*"([^"]+)",\s*"([^"]+)"/g)) {
      expect(extensionGlyphFor(m[2]), m[2]).toEqual(extensionGlyphFor(m[1]));
    }
    const display = meta.match(/fn bundled_extension_display_name[\s\S]*?\n\}/)?.[0] ?? '';
    for (const m of display.matchAll(/"([^"]+)"\s*=>\s*return Some\("([^"]+)"\)/g)) {
      expect(isBuiltInExtension(m[2]), m[2]).toBe(true);
      expect(extensionGlyphFor(m[2]), m[2]).toEqual(extensionGlyphFor(m[1]));
    }
  });
});

/**
 * A NEW tool on a built-in server must not ship with the generic Puzzle or the
 * wrench. Reads the servers' `#[tool(...)]` attributes and the platform tool
 * constants, so adding a tool without a glyph fails here.
 */
describe('every built-in server tool has a real glyph', () => {
  const MCP_SRC = join(CRATES, 'biorouter-mcp/src');
  const SERVER_DIRS: Record<string, string> = {
    developer: 'developer',
    knowledge: 'knowledge',
    memory: 'memory',
    webdocuments: 'webdocuments',
    autovisualiser: 'autovisualiser',
    agent_drafter: 'agent_drafter',
    datasql: 'datasql',
    files_server: 'files',
    compute_server: 'compute',
  };
  const TOOL_ATTR =
    /#\[tool\(([\s\S]*?)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/g;

  function rustFiles(dir: string): string[] {
    return readdirSync(dir).flatMap((entry) => {
      const path = join(dir, entry);
      if (statSync(path).isDirectory()) return rustFiles(path);
      return entry.endsWith('.rs') && !entry.includes('test') ? [path] : [];
    });
  }

  for (const [dir, extension] of Object.entries(SERVER_DIRS)) {
    it(`${dir}`, () => {
      expect(existsSync(join(MCP_SRC, dir)), `${dir} moved`).toBe(true);
      const names = new Set<string>();
      for (const file of rustFiles(join(MCP_SRC, dir))) {
        for (const m of readFileSync(file, 'utf8').matchAll(TOOL_ATTR)) {
          names.add(m[1].match(/name\s*=\s*"([^"]+)"/)?.[1] ?? m[2]);
        }
      }
      expect(names.size, `no #[tool] found under ${dir}: the scan is vacuous`).toBeGreaterThan(0);
      for (const tool of names) {
        const kind = toolGlyphKind(`${extension}__${tool}`);
        expect(['unknown', 'extension'], `${extension}__${tool}`).not.toContain(kind);
      }
    });
  }

  it('platform tools', () => {
    const src = read('biorouter/src/agents/platform_tools.rs');
    const names = [...src.matchAll(/_TOOL_NAME:\s*&str\s*=\s*"(platform__\w+)"/g)].map((m) => m[1]);
    expect(names.length).toBeGreaterThan(0);
    for (const name of names) {
      expect(['unknown', 'extension'], name).not.toContain(toolGlyphKind(name));
    }
  });
});
