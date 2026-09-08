import type React from 'react';
import { AppWindow, Clock, KnowledgeIcon, Layers, Pipeline, Puzzle } from './app-icons';

// One glyph per entity, in one place. A workflow is a Pipeline everywhere it is
// drawn — the sidebar row, the mention popover, the reset panel, the workflows
// list — and a knowledge base is always the KnowledgeIcon. Without this record
// the same entity picked up a different mark in each view (design.md §3.9 —
// "one glyph, one meaning").
export type EntityKind =
  | 'workflow'
  | 'knowledge'
  | 'extension'
  | 'skill'
  | 'application'
  | 'schedule';

export type EntityIcon = React.ComponentType<{
  className?: string;
  style?: React.CSSProperties;
}>;

export const ENTITY_ICONS: Record<EntityKind, EntityIcon> = {
  workflow: Pipeline,
  knowledge: KnowledgeIcon,
  extension: Puzzle,
  skill: Layers,
  application: AppWindow,
  schedule: Clock,
};
