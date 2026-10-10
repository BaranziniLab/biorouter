import * as React from 'react';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';
import { Search } from './icons/app-icons';
import { TranscriptRow } from './TranscriptRow';
import { exploreStepOf, type ExploreKind } from './ToolCallWithResponse';
import { EXPLORED_COPY } from './toolCallCopy';

/** One entry of a message's tool block: a call on its own line, or a folded run. */
export type ToolRowSegment<T> =
  | { type: 'single'; item: T }
  | { type: 'explored'; items: T[]; kinds: ExploreKind[]; running: boolean };

/** A run shorter than this keeps its own lines: one read is just a read. */
const MIN_EXPLORE_RUN = 2;

/**
 * Fold consecutive reads, lists and searches into one segment (D18). `stepOf`
 * answers whether an item may fold and as what; anything it refuses breaks the
 * run, so the order of the calls is never changed.
 */
export function foldExploreRuns<T>(
  items: readonly T[],
  stepOf: (item: T) => { kind: ExploreKind; running: boolean } | null
): ToolRowSegment<T>[] {
  const segments: ToolRowSegment<T>[] = [];
  let run: { item: T; kind: ExploreKind; running: boolean }[] = [];

  const flush = () => {
    if (run.length >= MIN_EXPLORE_RUN) {
      segments.push({
        type: 'explored',
        items: run.map((step) => step.item),
        kinds: run.map((step) => step.kind),
        running: run.some((step) => step.running),
      });
    } else {
      run.forEach((step) => segments.push({ type: 'single', item: step.item }));
    }
    run = [];
  };

  for (const item of items) {
    const step = stepOf(item);
    if (step) {
      run.push({ item, ...step });
    } else {
      flush();
      segments.push({ type: 'single', item });
    }
  }
  flush();
  return segments;
}

/** "3 files, 1 search" for a run's kinds. */
export function exploredCounts(kinds: readonly ExploreKind[]): string {
  const count = (kind: ExploreKind) => kinds.filter((k) => k === kind).length;
  return EXPLORED_COPY.counts({
    read: count('read'),
    list: count('list'),
    search: count('search'),
  });
}

/**
 * `[Search 16] Explored · 3 files, 1 search ›`: one line for a run of calls
 * that only looked around, opening onto the calls themselves (each still its
 * own row, 4px apart, indented to the label). "Exploring" while one of them is
 * still working, with the running pulse.
 */
export function ExploredRow({
  kinds,
  running,
  children,
}: {
  kinds: readonly ExploreKind[];
  running: boolean;
  children: React.ReactNode;
}) {
  return (
    <TranscriptRow
      icon={Search}
      glyph="explored"
      label={running ? EXPLORED_COPY.running : EXPLORED_COPY.done}
      meta={`· ${exploredCounts(kinds)}`}
      running={running}
      body="plain"
      data-testid="tool-explored-row"
    >
      <div className="flex min-w-0 flex-col gap-1">{children}</div>
    </TranscriptRow>
  );
}

/**
 * A message's tool block: one line per call, with runs of two or more reads,
 * lists and searches folded into one "Explored" row (spec 3.6, D18). Each
 * entry, folded or not, is a `.biorouter-message-tool`, so the live entrance
 * and the history-load rest apply to it alike.
 *
 * `renderCall` draws one call (the transcript passes its `ToolCallWithResponse`
 * with the message's props); this component only decides the grouping.
 */
export function ToolCallRows({
  toolRequests,
  toolResponses,
  turnActive,
  renderCall,
}: {
  toolRequests: readonly ToolRequestMessageContent[];
  toolResponses: ReadonlyMap<string, ToolResponseMessageContent>;
  turnActive: boolean;
  renderCall: (toolRequest: ToolRequestMessageContent) => React.ReactNode;
}) {
  const segments = foldExploreRuns(toolRequests, (toolRequest) =>
    exploreStepOf(toolRequest, toolResponses.get(toolRequest.id), turnActive)
  );
  return (
    <>
      {segments.map((segment) =>
        segment.type === 'single' ? (
          <div className="biorouter-message-tool" key={segment.item.id}>
            {renderCall(segment.item)}
          </div>
        ) : (
          // Keyed on the run's first call, so a run that grows while it is
          // open stays open.
          <div className="biorouter-message-tool" key={`explored-${segment.items[0].id}`}>
            <ExploredRow kinds={segment.kinds} running={segment.running}>
              {segment.items.map((toolRequest) => (
                <div key={toolRequest.id} className="min-w-0">
                  {renderCall(toolRequest)}
                </div>
              ))}
            </ExploredRow>
          </div>
        )
      )}
    </>
  );
}
