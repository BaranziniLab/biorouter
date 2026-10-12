import React from 'react';
import { isImageExtension } from '../utils/imageFormats';
import {
  File,
  Image,
  Video,
  Music,
  Package,
  Archive,
  FileText,
  Palette,
  Code,
  Database,
  Settings,
  SquareSlash,
  Terminal,
} from './icons/app-icons';
import { ENTITY_ICONS } from './icons/entity-icons';
import { DisplayItem } from './MentionPopover';

interface FileIconProps {
  item: DisplayItem;
}

interface IconInfo {
  Icon: React.ComponentType<{ className?: string; style?: React.CSSProperties }>;
  color: string;
}

// Icons are monochrome and inherit their colour from the surrounding text via
// `currentColor` (design.md §3.9 — "Colour: currentColor, always. Never a hex").
// The item *type* is conveyed by the glyph, not by a per-type tint. `color` is
// kept on the return shape so the public API is unchanged.
const CURRENT = 'currentColor';

export const getItemIcon = (item: DisplayItem): IconInfo => {
  switch (item.itemType) {
    // A built-in entry is a slash command: the one slash-command mark, not a
    // bolt (the bolt now lives inside the Skill glyph) and not `Terminal`.
    case 'Builtin':
      return { Icon: SquareSlash, color: CURRENT };
    case 'Workflow':
      return { Icon: ENTITY_ICONS.workflow, color: CURRENT };
    case 'KnowledgeBase':
      return { Icon: ENTITY_ICONS.knowledge, color: CURRENT };
    case 'Skill':
      return { Icon: ENTITY_ICONS.skill, color: CURRENT };
    case 'Extension':
      return { Icon: ENTITY_ICONS.extension, color: CURRENT };
    case 'Directory':
      return { Icon: ENTITY_ICONS.folder, color: CURRENT };
    default: {
      const ext = item.name.split('.').pop()?.toLowerCase() || '';

      // Image files
      // Shares the panel's list so this icon cannot promise a preview the panel
      // will refuse (it used to offer bmp/tiff/ico that landed on the binary card).
      if (isImageExtension(ext)) {
        return { Icon: Image, color: CURRENT };
      }

      // Video files
      if (['mp4', 'mov', 'avi', 'mkv', 'webm', 'flv', 'wmv'].includes(ext)) {
        return { Icon: Video, color: CURRENT };
      }

      // Audio files
      if (['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a'].includes(ext)) {
        return { Icon: Music, color: CURRENT };
      }

      // Archive/compressed files
      if (['zip', 'tar', 'gz', 'rar', '7z', 'bz2'].includes(ext)) {
        return { Icon: Archive, color: CURRENT };
      }

      // PDF files
      if (ext === 'pdf') {
        return { Icon: FileText, color: CURRENT };
      }

      // Design files
      if (['ai', 'eps', 'sketch', 'fig', 'xd', 'psd'].includes(ext)) {
        return { Icon: Palette, color: CURRENT };
      }

      // JavaScript/TypeScript files
      if (['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs'].includes(ext)) {
        return { Icon: Code, color: CURRENT };
      }

      // Python files
      if (['py', 'pyw', 'pyc'].includes(ext)) {
        return { Icon: Code, color: CURRENT };
      }

      // HTML files
      if (['html', 'htm', 'xhtml'].includes(ext)) {
        return { Icon: Code, color: CURRENT };
      }

      // CSS files
      if (['css', 'scss', 'sass', 'less', 'stylus'].includes(ext)) {
        return { Icon: Code, color: CURRENT };
      }

      // JSON/Data files
      if (['json', 'xml', 'yaml', 'yml', 'toml', 'csv'].includes(ext)) {
        return { Icon: FileText, color: CURRENT };
      }

      // Markdown files
      if (['md', 'markdown', 'mdx'].includes(ext)) {
        return { Icon: FileText, color: CURRENT };
      }

      // Database files
      if (['sql', 'db', 'sqlite', 'sqlite3'].includes(ext)) {
        return { Icon: Database, color: CURRENT };
      }

      // Configuration files
      if (
        [
          'env',
          'ini',
          'cfg',
          'conf',
          'config',
          'gitignore',
          'dockerignore',
          'editorconfig',
          'prettierrc',
          'eslintrc',
        ].includes(ext || '') ||
        ['dockerfile', 'makefile', 'rakefile', 'gemfile'].includes(item.name.toLowerCase())
      ) {
        return { Icon: Settings, color: CURRENT };
      }

      // Text files
      if (
        ['txt', 'log', 'readme', 'license', 'changelog', 'contributing'].includes(ext || '') ||
        ['readme', 'license', 'changelog', 'contributing'].includes(item.name.toLowerCase())
      ) {
        return { Icon: FileText, color: CURRENT };
      }

      // Executables and installers: a package. Not the wrench, which means
      // "a tool call nothing recognises" and nothing else.
      if (['exe', 'app', 'deb', 'rpm', 'dmg', 'pkg', 'msi'].includes(ext || '')) {
        return { Icon: Package, color: CURRENT };
      }

      // Script files
      if (
        ['sh', 'bash', 'zsh', 'fish', 'bat', 'cmd', 'ps1', 'rb', 'pl', 'php'].includes(ext || '')
      ) {
        return { Icon: Terminal, color: CURRENT };
      }

      // Default file icon
      return { Icon: File, color: CURRENT };
    }
  }
};

export const ItemIcon: React.FC<FileIconProps> = ({ item }) => {
  const { Icon, color } = getItemIcon(item);

  return <Icon className="w-4 h-4" style={{ color }} />;
};
