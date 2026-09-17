import React from 'react';
import {
  Archive,
  BookMarked,
  Bookmark,
  BookmarkPlus,
  Camera,
  CheckCircle,
  Code,
  Edit,
  Eye,
  FilePlus,
  FileSpreadsheet,
  FileText,
  Globe,
  Monitor,
  Search,
  Terminal,
  Wrench,
} from '../components/icons/app-icons';

export type ToolIconProps = {
  className?: string;
};

/**
 * Maps tool names to their corresponding icon components
 * @param toolName - The name of the tool (extracted from toolCall.name)
 * @returns React component for the tool icon
 */
export const getToolIcon = (toolName: string): React.ComponentType<ToolIconProps> => {
  switch (toolName) {
    // Developer Extension Tools
    case 'text_editor':
      return Edit;
    case 'shell':
      return Terminal;

    // Memory Extension Tools
    case 'remember_memory':
      return BookmarkPlus;
    case 'retrieve_memories':
      return BookMarked;

    // Computer Use Extension Tools
    case 'list_apps':
    case 'get_app_state':
    case 'click':
    case 'perform_secondary_action':
    case 'scroll':
    case 'drag':
    case 'type_text':
    case 'press_key':
    case 'set_value':
      return Monitor;
    case 'web_scrape':
      return Globe;
    case 'screen_capture':
      return Camera;
    case 'pdf_tool':
      return FileText;
    case 'docx_tool':
      return FileText;
    case 'xlsx_tool':
      return FileSpreadsheet;
    case 'cache':
      return Archive;

    // File Operations
    case 'search':
      return Search;
    case 'read':
      return Eye;
    case 'create_file':
      return FilePlus;
    case 'update_file':
      return Edit;

    // Google Workspace Tools (if still supported)
    case 'sheets_tool':
      return FileSpreadsheet;
    case 'docs_tool':
      return FileText;

    // Special Tools
    case 'final_output':
      return CheckCircle;

    // Default fallback for unknown tools
    default:
      return Wrench;
  }
};

/**
 * Maps extension names to their corresponding icon components
 * @param extensionName - The name of the extension
 * @returns React component for the extension icon
 */
export const getExtensionIcon = (extensionName: string): React.ComponentType<ToolIconProps> => {
  switch (extensionName) {
    case 'developer':
      return Code;
    case 'memory':
      return Bookmark;
    case 'computercontroller':
      return Monitor;
    case 'webdocuments':
      return Globe;
    default:
      return Wrench;
  }
};

/**
 * Helper function to extract tool name from full tool call name
 * @param toolCallName - Full tool call name (e.g., "developer__text_editor")
 * @returns Extracted tool name (e.g., "text_editor")
 */
export const extractToolName = (toolCallName: string): string => {
  return toolCallName.substring(toolCallName.lastIndexOf('__') + 2);
};

/**
 * Helper function to extract extension name from full tool call name
 * @param toolCallName - Full tool call name (e.g., "developer__text_editor")
 * @returns Extracted extension name (e.g., "developer")
 */
export const extractExtensionName = (toolCallName: string): string => {
  const lastIndex = toolCallName.lastIndexOf('__');
  return lastIndex === -1 ? '' : toolCallName.substring(0, lastIndex);
};

/**
 * Main function to get the appropriate icon for a tool call
 * @param toolCallName - Full tool call name (e.g., "developer__text_editor")
 * @param useExtensionIcon - Whether to use extension icon instead of tool icon
 * @returns React component for the icon
 */
export const getToolCallIcon = (
  toolCallName: string,
  useExtensionIcon: boolean = false
): React.ComponentType<ToolIconProps> => {
  if (useExtensionIcon) {
    const extensionName = extractExtensionName(toolCallName);
    return getExtensionIcon(extensionName);
  }

  const toolName = extractToolName(toolCallName);
  return getToolIcon(toolName);
};
