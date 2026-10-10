/**
 * Shim over `utils/toolGlyph.ts`, kept so the callers that still import this
 * module (`ToolCallWithResponse`, `PendingToolCallCard`) compile unchanged
 * until they move to `toolGlyphFor(name, args)`. Every answer comes from the
 * one ladder in `toolGlyph.ts`; nothing here decides a glyph of its own.
 *
 * ⚠ The old module keyed on the BARE tool name (everything after the last
 * `__`), which is why most built-in tool rows drew the wrench and two
 * extensions' same-named tools collided. That behaviour is gone: these
 * functions read the full name.
 */
import type React from 'react';
import { extensionGlyphFor, parseToolName, toolGlyphFor } from './toolGlyph';

export type ToolIconProps = {
  className?: string;
};

/** The glyph for a tool call by its full name (`developer__shell`). The
 * optional arguments let `text_editor` tell a view from an edit. */
export const getToolIcon = (
  toolName: string,
  args?: Record<string, unknown> | null
): React.ComponentType<ToolIconProps> => toolGlyphFor(toolName, args).Icon;

/** The glyph an extension draws, by key or display name. */
export const getExtensionIcon = (extensionName: string): React.ComponentType<ToolIconProps> =>
  extensionGlyphFor(extensionName).Icon;

/** `developer__text_editor` -> `text_editor`. */
export const extractToolName = (toolCallName: string): string => parseToolName(toolCallName).tool;

/** `developer__text_editor` -> `developer`; '' for a bare name. */
export const extractExtensionName = (toolCallName: string): string =>
  parseToolName(toolCallName).extension;

/**
 * The glyph for a tool call row. `useExtensionIcon` draws the extension's own
 * glyph instead of the tool's.
 */
export const getToolCallIcon = (
  toolCallName: string,
  useExtensionIcon: boolean = false,
  args?: Record<string, unknown> | null
): React.ComponentType<ToolIconProps> =>
  useExtensionIcon
    ? getExtensionIcon(extractExtensionName(toolCallName))
    : getToolIcon(toolCallName, args);
