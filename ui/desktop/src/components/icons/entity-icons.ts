import type React from 'react';
import {
  Agent,
  AppWindow,
  Brain,
  ChatBubble,
  Clock,
  Folder,
  KnowledgeGraph,
  Pipeline,
  Puzzle,
  Skill,
  Users,
} from './app-icons';

// One glyph per entity, in one place. A workflow is a Pipeline everywhere it is
// drawn (the sidebar row, the mention popover, the reset panel, the workflows
// list, a workflow tool row) and a knowledge base is always the knowledge graph.
// Without this record the same entity picked up a different mark in each view
// (design.md §3.9: "one glyph, one meaning").
//
// The chat kinds and the tool-call glyphs (`utils/toolGlyph.ts`) read their
// component glyphs from here, and their tests assert the identity, so a
// knowledge tool row, the Knowledge nav row and the composer's knowledge count
// can never disagree.
export type EntityKind =
  | 'workflow'
  | 'knowledge'
  | 'extension'
  | 'skill'
  | 'application'
  | 'schedule'
  | 'crew'
  | 'agent'
  | 'chat'
  | 'model'
  | 'folder';

export type EntityIcon = React.ComponentType<{
  className?: string;
  style?: React.CSSProperties;
}>;

export const ENTITY_ICONS: Record<EntityKind, EntityIcon> = {
  workflow: Pipeline,
  knowledge: KnowledgeGraph,
  extension: Puzzle,
  skill: Skill,
  application: AppWindow,
  // Also the scheduled-run chat glyph (it was CalendarClock there).
  schedule: Clock,
  crew: Users,
  agent: Agent,
  chat: ChatBubble,
  model: Brain,
  // A working folder. "Open in Finder" is `FolderOpen`, an action, not this.
  folder: Folder,
};
