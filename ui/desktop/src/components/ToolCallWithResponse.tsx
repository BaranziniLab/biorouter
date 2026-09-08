import { ToolIconWithStatus, ToolCallStatus } from './ToolCallStatusIndicator';
import { getToolCallIcon } from '../utils/toolIconMapping';
import React, { useEffect, useRef, useState } from 'react';
import { Prism as SyntaxHighlighter } from 'react-syntax-highlighter';
import { useResolvedTheme, useThemeFamily } from '../contexts/ThemeContext';
import {
  CODE_FONT_FAMILY,
  CODE_FONT_SIZE,
  CODE_LINE_HEIGHT,
  codeThemesByFamily,
} from '../styles/codeTheme';
import { Button } from './ui/button';
import { ToolCallArguments, ToolCallArgumentValue } from './ToolCallArguments';
import MarkdownContent from './MarkdownContent';
import {
  ToolRequestMessageContent,
  ToolResponseMessageContent,
  NotificationEvent,
} from '../types/message';
import { cn, toolIdentifierToTitleCase } from '../utils';
import { LoadingStatus } from './ui/Dot';
import { ChevronRight } from './icons/app-icons';
import MCPUIResourceRenderer from './MCPUIResourceRenderer';
import { isUIResource } from '@mcp-ui/client';
import { CallToolResponse, Content, EmbeddedResource } from '../api';
import type { ArtifactSource } from './artifacts/artifactTypes';
import { NotificationContent, NotificationSurface } from './alerts/NotificationSurface';
import { crossAffiliationOffer } from '../utils/crossAffiliation';
import { CrossAffiliationAcceptCard } from './privacy/CrossAffiliationAcceptCard';
import { unwrapGuardrailFrameInContent } from '../utils/guardrailFrame';

/**
 * The tool card's own status vocabulary. `interrupted` extends the shared
 * `LoadingStatus` for the one case only a tool card has: the turn ended without
 * a tool response ever arriving. It renders through the (previously
 * unreachable) `pending` dot rather than claiming success.
 */
type ToolLoadingStatus = LoadingStatus | 'interrupted';

interface ToolGraphNode {
  tool: string;
  description: string;
  depends_on: number[];
}

/**
 * One executed sub-call of a coordinated `execute_code` step (#28), recorded
 * by the backend in the result meta under `biorouter/tool-calls`.
 */
type ExecutedToolCall = {
  tool: string;
  /** The exact JSON arguments string (backend-truncated at ~2 KB). */
  args?: string;
  status: 'ok' | 'error';
  /** The real error text for a failed call. */
  error?: string;
  resultBytes?: number;
  todoTask?: { id: string; title: string };
};

const MAX_EXECUTED_ARGUMENT_BYTES = 2048 + 3; // Backend cap plus a UTF-8 ellipsis.
const MAX_EXECUTED_TASK_ID_BYTES = 64;
const MAX_EXECUTED_TASK_TITLE_BYTES = 512;

type ResultMeta = {
  'biorouter/tool-calls'?: unknown;
  'biorouter/tool-calls-dropped'?: unknown;
};

type ToolResultWithMeta = {
  status?: string;
  value?: CallToolResponse & {
    _meta?: ResultMeta;
  };
};

interface ToolCallWithResponseProps {
  sessionId?: string;
  isCancelledMessage: boolean;
  toolRequest: ToolRequestMessageContent;
  toolResponse?: ToolResponseMessageContent;
  notifications?: NotificationEvent[];
  isStreamingMessage?: boolean;
  /**
   * Whether the CHAT TURN is still running (chat-level `chatState !== Idle`),
   * as opposed to `isStreamingMessage`, which only says "this message is
   * currently the last one". Tool status must key off the turn: a tool call
   * keeps executing long after its message stops being last. Defaults to
   * false so a read-only replay never spins.
   */
  turnActive?: boolean;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}

function recordOf(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null;
}

function normalizeToolGraph(value: unknown): ToolGraphNode[] {
  if (!Array.isArray(value)) return [];

  return value.flatMap((candidate) => {
    const node = recordOf(candidate);
    if (!node) return [];

    const dependencies = Array.isArray(node.depends_on)
      ? node.depends_on.filter(
          (dependency): dependency is number =>
            typeof dependency === 'number' && Number.isInteger(dependency) && dependency >= 0
        )
      : [];

    return [
      {
        tool: typeof node.tool === 'string' && node.tool.trim() ? node.tool : 'Unspecified tool',
        description:
          typeof node.description === 'string' && node.description.trim()
            ? node.description
            : 'No description was provided.',
        depends_on: dependencies,
      },
    ];
  });
}

function isBoundedMetadataText(value: unknown, maxBytes: number): value is string {
  if (typeof value !== 'string' || !value.length || value.length > maxBytes) return false;
  let bytes = 0;
  for (const character of value) {
    const point = character.codePointAt(0) ?? 0;
    if (point >= 0xd800 && point <= 0xdfff) return false;
    bytes += point <= 0x7f ? 1 : point <= 0x7ff ? 2 : point <= 0xffff ? 3 : 4;
    if (bytes > maxBytes) return false;
  }
  return true;
}

function normalizeExecutedTodoTask(record: Record<string, unknown>): ExecutedToolCall['todoTask'] {
  if (
    record.tool !== 'todo__todo_update' ||
    record.status !== 'ok' ||
    !isBoundedMetadataText(record.args, MAX_EXECUTED_ARGUMENT_BYTES)
  ) {
    return undefined;
  }
  const args = parsedCallArguments(record.args);
  const task = recordOf(record.todo_task);
  if (!args || Array.isArray(args) || typeof args.id !== 'string' || !task || Array.isArray(task)) {
    return undefined;
  }
  const id = args.id.replace(/^#/, '');
  if (
    !isBoundedMetadataText(id, MAX_EXECUTED_TASK_ID_BYTES) ||
    task.id !== id ||
    !isBoundedMetadataText(task.title, MAX_EXECUTED_TASK_TITLE_BYTES) ||
    !task.title.trim()
  ) {
    return undefined;
  }
  return { id, title: task.title };
}

/**
 * Executed sub-call telemetry from the result meta (#28). The meta is untyped
 * wire data, so anything malformed is dropped rather than crashing the card.
 */
function normalizeExecutedCalls(value: unknown): ExecutedToolCall[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((candidate): ExecutedToolCall[] => {
    const record = recordOf(candidate);
    if (!record || typeof record.tool !== 'string' || !record.tool.trim()) return [];
    return [
      {
        tool: record.tool,
        args: typeof record.args === 'string' && record.args.trim() ? record.args : undefined,
        status: record.status === 'error' ? 'error' : 'ok',
        error: typeof record.error === 'string' && record.error.trim() ? record.error : undefined,
        resultBytes: typeof record.result_bytes === 'number' ? record.result_bytes : undefined,
        todoTask: normalizeExecutedTodoTask(record),
      },
    ];
  });
}

/**
 * The reserved per-tool provider-metadata key a coding-agent provider stamps on
 * a **mirrored** tool call — one the child agent already executed, replayed into
 * the transcript so the ordinary card can draw it.
 *
 * ⚠ Must match `mirror::PROVIDER_EXECUTED_KEY` in
 * `crates/biorouter/src/providers/coding_agent/mirror.rs`. It rides
 * `ToolRequest.metadata` / `ToolResponse.metadata`, which are already part of
 * the serialized schema, so nothing here needs a generated type.
 */
const PROVIDER_EXECUTED_KEY = 'biorouterProviderExecuted';

/**
 * Who ran a mirrored call, and therefore which guarantees hold.
 *
 * `bridged` — it crossed Biorouter's tool bridge and passed its inspectors,
 * permission mode, `.biorouterignore`, vault and privacy Gate C. Identical in
 * every way to a call an API provider made, so it MUST render identically: no
 * badge, no extra chrome.
 *
 * `child` — it ran inside the child agent's own sandbox (Codex's
 * `exec`/`apply_patch`, or an MCP server configured in the user's own
 * `~/.codex/config.toml`) and passed NONE of those gates. That is the one
 * honest difference in this feature, and the card says so.
 */
export type ProviderExecution = 'bridged' | 'child';

/**
 * Read the mirror marker off a request/response pair.
 *
 * Both halves are stamped, so a row read on its own carries its own provenance;
 * Both halves are stamped, so a row read on its own carries its own provenance.
 * An unrecognised value is `null` — the same fail-safe direction the backend
 * takes: we only claim a provenance we positively recognise.
 *
 * ⚠ **If the two halves disagree, `child` wins.** This is a safety label, so the
 * fail-safe direction is to under-claim: saying "BioRouter gated this" about a
 * call it never inspected is the one thing this label must never do, whereas an
 * unnecessary "not gated" note is merely noise. Today's decoders stamp one
 * execution kind on both halves, so a disagreement means something is already
 * wrong — which is exactly when the cautious reading matters.
 */
export function providerExecutionOf(
  toolRequest?: Pick<ToolRequestMessageContent, 'metadata'>,
  toolResponse?: Pick<ToolResponseMessageContent, 'metadata'>
): ProviderExecution | null {
  let seen: ProviderExecution | null = null;
  for (const carrier of [toolRequest, toolResponse]) {
    const value = carrier?.metadata?.[PROVIDER_EXECUTED_KEY];
    if (value === 'child') return 'child';
    if (value === 'bridged') seen = 'bridged';
  }
  return seen;
}

/**
 * The full sentence behind the row's short label. Kept out of the visible row
 * because D-17 says a tool call is a LINE, not a card — the row states the fact,
 * the tooltip explains it.
 */
const CHILD_EXECUTED_TITLE =
  "This tool ran inside the coding agent's own sandbox. Biorouter's inspectors, " +
  'permission mode, .biorouterignore and privacy gates did not apply to it.';

function normalizedToolResultValue(toolResult: unknown): Record<string, unknown> | null {
  const result = recordOf(toolResult);
  if (!result) return null;
  if (result.status === 'error') return null;
  if (result.status === 'success') return recordOf(result.value);
  return result;
}

function getToolResultContent(toolResult: unknown): Content[] {
  const value = normalizedToolResultValue(toolResult);
  if (!Array.isArray(value?.content)) {
    return [];
  }

  return (
    value.content
      .filter((item): item is Content => {
        if (!recordOf(item)) return false;
        const annotations = (item as { annotations?: { audience?: string[] } }).annotations;
        return !annotations?.audience || annotations.audience.includes('user');
      })
      // Every surface that shows a person tool text reads it through here — the
      // markdown view, the resource JSON dump, the error banner, and the saved
      // and shared transcript replays that reuse this component. So the
      // untrusted-data frame the backend wraps around every result comes off
      // here, once, rather than at four call sites that would drift. Only the
      // frame: a `[BIOROUTER GUARDRAIL]` warning above it is the user's to read
      // and is left in place. Nothing on the model's path passes through this
      // function.
      .map(unwrapGuardrailFrameInContent)
  );
}

function displayError(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error;
  const record = recordOf(error);
  if (typeof record?.message === 'string' && record.message.trim()) return record.message;
  try {
    const serialized = JSON.stringify(error);
    if (serialized && serialized !== '{}') return serialized;
  } catch {
    return 'The tool returned an unreadable error.';
  }
  return 'The tool call failed without an error message.';
}

function getToolResultError(toolResult: unknown): string | null {
  const result = recordOf(toolResult);
  if (!result) return null;
  if (result.status === 'error') return displayError(result.error);

  const value = normalizedToolResultValue(result);
  // `isError` is the wire spelling: rmcp serialises CallToolResult with
  // rename_all = "camelCase", so every live and persisted tool result uses it.
  // `is_error` is kept for tolerance of any snake_case producer.
  if (value?.isError !== true && value?.is_error !== true) return null;
  const text = getToolResultContent(result)
    .flatMap((content) =>
      'text' in content && typeof content.text === 'string' ? [content.text] : []
    )
    .join('\n')
    .trim();
  if (text) return text;
  // No user-audience error text exists. Content marked assistant-only was
  // deliberately kept out of the user's view by the tool, so the audience
  // filter holds even on the error path: falling back to assistant-only text
  // here would leak content the user was never meant to see. Backends that
  // want a specific message must supply user-visible error text.
  return 'The tool reported that it could not complete the request.';
}

function isEmbeddedResource(content: Content): content is EmbeddedResource {
  return 'resource' in content && typeof (content as Record<string, unknown>).resource === 'object';
}

interface ToolCallRenderBoundaryProps {
  children: React.ReactNode;
  resetValue: unknown;
}

class ToolCallRenderBoundary extends React.Component<
  ToolCallRenderBoundaryProps,
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidUpdate(previousProps: ToolCallRenderBoundaryProps) {
    if (this.state.error && previousProps.resetValue !== this.props.resetValue) {
      this.setState({ error: null });
    }
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <NotificationSurface
        status="error"
        role="alert"
        title="Tool details unavailable"
        message="This tool returned an unexpected response. The chat can continue."
      >
        <details className="mt-2.5 text-xs text-text-muted">
          <summary className="w-fit cursor-pointer select-none font-medium text-text-default">
            Technical details
          </summary>
          <div className="mt-2 whitespace-pre-wrap break-words font-mono [overflow-wrap:anywhere]">
            {this.state.error.message}
          </div>
        </details>
      </NotificationSurface>
    );
  }
}

export default function ToolCallWithResponse(props: ToolCallWithResponseProps) {
  return (
    <ToolCallRenderBoundary resetValue={props.toolResponse ?? props.toolRequest}>
      <ToolCallWithResponseContent {...props} />
    </ToolCallRenderBoundary>
  );
}

function ToolCallWithResponseContent({
  sessionId,
  isCancelledMessage,
  toolRequest,
  toolResponse,
  notifications,
  isStreamingMessage,
  turnActive = false,
  onOpenArtifact,
  workingDir,
}: ToolCallWithResponseProps) {
  // Handle both the wrapped ToolResult format and the unwrapped format
  // The server serializes ToolResult<T> as { status: "success", value: T } or { status: "error", error: string }
  const toolCallData = toolRequest.toolCall as Record<string, unknown>;
  const toolCall =
    toolCallData?.status === 'success'
      ? (toolCallData.value as { name: string; arguments: Record<string, unknown> })
      : (toolCallData as { name: string; arguments: Record<string, unknown> });

  if (!toolCall || !toolCall.name) {
    return null;
  }

  const isError = getToolResultError(toolResponse?.toolResult) !== null;

  return (
    <>
      {/* D-17: a tool call is a LINE in the transcript, not a card. An outline
          around every one of them reads as a focus ring and turns a quiet
          transcript into a stack of boxes. Failure is signalled by colour — the
          status icon and label — plus a faint danger wash, never by an outline. */}
      <div
        className={cn(
          'w-full overflow-hidden rounded-md text-sm font-sans transition-colors',
          isError && 'bg-background-danger/5'
        )}
      >
        <ToolCallView
          {...{
            isCancelledMessage,
            sessionId,
            toolCall,
            toolResponse,
            providerExecution: providerExecutionOf(toolRequest, toolResponse),
            notifications,
            isStreamingMessage,
            turnActive,
            onOpenArtifact,
            workingDir,
          }}
        />
      </div>
      {/* A `ui://` resource — an Auto Visualiser figure, an Agent Drafter app
          card. It is NEVER drawn here: MCPUIResourceRenderer emits a
          click-to-open card and the artifact side panel is the only surface
          that renders the thing itself. */}
      {toolResponse?.toolResult &&
        getToolResultContent(toolResponse.toolResult).map((content, index) => {
          const resourceContent = isEmbeddedResource(content)
            ? { ...content, type: 'resource' as const }
            : null;
          if (resourceContent && isUIResource(resourceContent)) {
            return (
              <div key={index} className="mt-3">
                <MCPUIResourceRenderer content={resourceContent} onOpenArtifact={onOpenArtifact} />
              </div>
            );
          } else {
            return null;
          }
        })}
    </>
  );
}

/**
 * ONE inset grid for everything a tool call opens into.
 *
 * Six met inside this one component — `pr-4 pl-6 pb-2`, `px-4 py-2`, `pl-4 pr-4
 * pb-2`, `pl-6 pr-2 pb-2`, `pl-4 pr-4 py-4` and `p-4` — so expanding two
 * different tool rows in the same transcript stepped the content in by two
 * different amounts, and nesting a disclosure inside a disclosure compounded the
 * disagreement. 12px on the sides and the bottom, 4px on top because the
 * disclosure row above has already opened the gap.
 */
const TOOL_INTERIOR_CLASS = 'px-3 pt-1 pb-3';

/**
 * ONE label recipe for a nested disclosure row. Was `pl-2 font-sans text-sm
 * text-text-muted` in four places and `pl-2 py-1 font-sans text-sm …` in two
 * more — the same sentence with a different vertical rhythm depending on which
 * one you opened.
 */
const TOOL_DISCLOSURE_LABEL_CLASS = 'pl-2 text-secondary text-text-muted';

interface ToolCallExpandableProps {
  label: string | React.ReactNode;
  isStartExpanded?: boolean;
  isForceExpand?: boolean;
  children: React.ReactNode;
  className?: string;
}

function ToolCallExpandable({
  label,
  isStartExpanded = false,
  isForceExpand,
  children,
  className = '',
}: ToolCallExpandableProps) {
  const [isExpandedState, setIsExpanded] = React.useState<boolean | null>(null);
  const isExpanded = isExpandedState === null ? isStartExpanded : isExpandedState;
  const toggleExpand = () => setIsExpanded(!isExpanded);
  React.useEffect(() => {
    if (isForceExpand) setIsExpanded(true);
  }, [isForceExpand]);

  return (
    <div className={className}>
      <Button
        onClick={toggleExpand}
        className="group h-6 min-h-0 max-w-full justify-start !px-0 py-0 text-left transition-colors rounded-md hover:bg-transparent focus-visible:bg-transparent"
        variant="ghost"
        size="xs"
      >
        <span className="flex min-w-0 max-w-full items-center overflow-hidden font-sans text-sm leading-6">
          {label}
        </span>
        {/* VISIBLE AT REST. It used to be `opacity-0` until hover, so nothing on
            a freshly loaded transcript said these rows open at all — a first-run
            user had to discover the whole tool detail view by accidentally
            hovering it. It is now muted-but-present, brightening on hover and on
            keyboard focus, and it takes the row's 16px icon size rather than
            14px so it does not sit in a cluster with a 16px tool glyph at a
            different scale (§3.8b: never two sizes in one cluster). */}
        <ChevronRight
          className={cn(
            'ml-1.5 size-4 shrink-0 text-text-muted opacity-60',
            'transition-[opacity,transform] group-hover:opacity-100 group-focus-visible:opacity-100',
            isExpanded && 'rotate-90 opacity-100'
          )}
        />
      </Button>
      {isExpanded && <div>{children}</div>}
    </div>
  );
}

interface ToolCallViewProps {
  isCancelledMessage: boolean;
  /**
   * Issue #56 DR-26 / Task 57: the chat a cross-institutional acceptance would
   * be keyed on. Absent on read-only surfaces that render a transcript with no
   * live session, where there is nothing to accept against.
   */
  sessionId?: string;
  toolCall: {
    name: string;
    arguments: Record<string, unknown>;
  };
  toolResponse?: ToolResponseMessageContent;
  notifications?: NotificationEvent[];
  /**
   * Retained for callers, but deliberately NOT read when deriving status: "this
   * message is currently last" is exactly the signal that used to paint pending
   * tool calls green. Use `turnActive` instead.
   */
  isStreamingMessage?: boolean;
  turnActive?: boolean;
  /**
   * Set only for a MIRRORED call — one a coding-agent provider replayed after
   * its child had already run it. `null` for every ordinary provider, and
   * `'bridged'` renders exactly as `null` does by design (see
   * {@link ProviderExecution}).
   */
  providerExecution?: ProviderExecution | null;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}

interface Progress {
  progress: number;
  progressToken: string;
  total?: number;
  message?: string;
}

export type ToolCallSummaryInput = {
  name: string;
  arguments?: Record<string, unknown>;
};

const MAX_SUMMARY_VALUE_LENGTH = 96;

const humanize = toolIdentifierToTitleCase;

const stringifySummaryValue = (value: unknown): string => {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (value === null || value === undefined) return '';
  return JSON.stringify(value);
};

const compactValue = (value: unknown, maxLength = MAX_SUMMARY_VALUE_LENGTH): string => {
  const text = stringifySummaryValue(value).replace(/\s+/g, ' ').trim();
  if (text.length <= maxLength) return text;
  const keep = Math.max(12, Math.floor((maxLength - 1) / 2));
  return `${text.slice(0, keep)}…${text.slice(-keep)}`;
};

const toolGraphNodeLabel = ({ tool, description }: ToolGraphNode): string => {
  const text = description.replace(/\s+/g, ' ').trim();
  const placeholder =
    text === 'No description was provided.' ||
    /^(?:tool(?: call)?|step|update|updating|task|operation)(?:\s+(?:tool(?: call)?|step|update|updating|task|operation))?\s*(?:(?:number|no\.?)\s*)?#?\s*\d+(?:\s+of\s+\d+)?[.!]?$/i.test(
      text
    );
  return placeholder
    ? humanize(tool.replace(/(?:__|[/\\])/g, '_'))
        .replace(/\s+/g, ' ')
        .trim()
    : text;
};

const summarizeToolGraph = (graph: ToolGraphNode[]): string => {
  const descriptions = graph.map(toolGraphNodeLabel);
  const actions = [...new Set(descriptions)];
  const shorten = (text: string, limit: number): string => {
    const characters = Array.from(text);
    return characters.length <= limit ? text : `${characters.slice(0, limit - 1).join('')}…`;
  };
  if (actions.length === 1) return shorten(actions[0], MAX_SUMMARY_VALUE_LENGTH);
  const actionLimit = Math.floor((MAX_SUMMARY_VALUE_LENGTH - 3) / 2);
  return `${shorten(actions[0], actionLimit)} → ${shorten(actions[actions.length - 1], actionLimit)}`;
};

const basename = (value: unknown): string => {
  const text = compactValue(value);
  const parts = text.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] || text;
};

const firstPresent = (args: Record<string, unknown>, keys: string[]): unknown => {
  for (const key of keys) {
    if (args[key] !== undefined && args[key] !== null && args[key] !== '') {
      return args[key];
    }
  }
  return undefined;
};

/** Longest task snippet a delegation label carries before it is elided. */
const MAX_DELEGATION_LABEL = 64;

/** The opening sentence of `text`, trimmed to a label-sized snippet. */
const headline = (text: string): string => {
  const firstSentence = text.split(/(?<=[.!?])\s/)[0] ?? text;
  // A very short leading fragment ("Do this.") says less than the whole
  // instruction would, so fall back to the full text in that case.
  const candidate = firstSentence.length >= 12 ? firstSentence : text;
  if (candidate.length <= MAX_DELEGATION_LABEL) return candidate;
  return `${candidate.slice(0, MAX_DELEGATION_LABEL - 1).trimEnd()}…`;
};

/**
 * A label for a `subagent` call that names THIS delegation.
 *
 * Every subagent call carries the same argument *keys*, so the generic
 * "<Tool> with <keys>" fallback rendered each one as the byte-identical
 * "Subagent with instructions". Tool cards start collapsed, so a turn that fans
 * out to five subagents was five identical rows: the user could not tell which
 * result came from which without expanding all of them — the one thing a
 * transcript of delegated work has to get right.
 */
const summarizeDelegation = (args: Record<string, unknown>): string => {
  const workflow = compactValue(firstPresent(args, ['subworkflow']) ?? '');
  const instructions = stringifySummaryValue(firstPresent(args, ['instructions']) ?? '')
    .replace(/\s+/g, ' ')
    .trim();
  const task = instructions ? headline(instructions) : '';
  if (workflow && task) return `Delegating ${workflow}: ${task}`;
  if (workflow) return `Delegating ${workflow}`;
  if (task) return `Delegating: ${task}`;
  return 'Delegating to a subagent';
};

const summarizeSearchQuery = (value: unknown): string | null => {
  if (!value) return null;
  if (typeof value === 'string') return compactValue(value);
  if (Array.isArray(value)) {
    // A plain list of terms (e.g. search_modules' `terms: ["fetch", "http"]`)
    // reads best joined, not JSON-stringified.
    if (
      value.length > 0 &&
      value.every((item) => typeof item === 'string' || typeof item === 'number')
    ) {
      return compactValue(value.join(', '));
    }
    const first = value[0] as Record<string, unknown> | undefined;
    return first ? compactValue(first.q ?? first.query ?? first.search_query ?? value) : null;
  }
  if (typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return compactValue(record.q ?? record.query ?? record.search_query ?? value);
  }
  return compactValue(value);
};

const namedArgument = (args: Record<string, unknown>, keys: string[]): string | null => {
  const value = firstPresent(args, keys);
  return value === undefined ? null : compactValue(value);
};

const namedEntity = (args: Record<string, unknown>, keys: string[]): string | null => {
  const value = namedArgument(args, keys);
  if (!value || /\s/.test(value)) return value;
  const versioned = value.match(/^(.+?)[-_]v?(\d+\.\d+\.\d+(?:-[\w.-]+)?(?:\+[\w.-]+)?)$/);
  const name = versioned?.[1] ?? value;
  const label = humanize(
    name.replace(/^(spoke|playwright|codegraph|cdw|ucsfomop)agent$/i, '$1_agent')
  );
  return versioned ? `${label} v${versioned[2]}` : label;
};

const countNamedArguments = (args: Record<string, unknown>, key: string): number =>
  Array.isArray(args[key]) ? args[key].length : 0;

const todoTaskTitle = (args: Record<string, unknown>, toolResult: unknown): string | null => {
  const replacement = namedArgument(args, ['text']);
  if (replacement) return replacement;
  if (getToolResultError(toolResult)) return null;
  const id = String(args.id ?? '').replace(/^#/, '');
  for (const content of getToolResultContent(toolResult)) {
    const item = recordOf(content);
    if (item?.type !== 'text' || typeof item.text !== 'string') continue;
    try {
      const task = recordOf(recordOf(JSON.parse(item.text))?.task);
      if (task?.id === id && typeof task.text === 'string' && task.text.trim()) {
        return compactValue(task.text);
      }
    } catch {
      // Older transcripts contain a plain-text acknowledgement, without task details.
    }
  }
  return null;
};

const compactExecutedTaskTitle = (title: string): string => {
  const characters = Array.from(title.replace(/\s+/g, ' ').trim());
  if (characters.length <= MAX_SUMMARY_VALUE_LENGTH) return characters.join('');
  const keep = Math.floor((MAX_SUMMARY_VALUE_LENGTH - 1) / 2);
  return `${characters.slice(0, keep).join('')}…${characters.slice(-keep).join('')}`;
};

const summarizeTodoCall = (
  toolName: string,
  args: Record<string, unknown>,
  toolResult: unknown,
  executedTitle?: string
): string | null => {
  if (toolName === 'plan_write') return 'Updating the work plan';
  if (toolName === 'todo_write') return 'Replacing the task list';
  if (toolName === 'todo_add') {
    const count = countNamedArguments(args, 'items');
    return count > 0 ? `Adding ${count} task${count === 1 ? '' : 's'}` : 'Adding tasks';
  }
  if (toolName === 'todo_expand') {
    const id = namedArgument(args, ['id']);
    const task = id ? `task ${id.startsWith('#') ? id : `#${id}`}` : 'a task';
    const count = countNamedArguments(args, 'items');
    return count > 0
      ? `Breaking ${task} into ${count} step${count === 1 ? '' : 's'}`
      : `Breaking ${task} into steps`;
  }
  if (toolName !== 'todo_update') return null;

  const id = namedArgument(args, ['id']);
  const title = executedTitle ?? todoTaskTitle(args, toolResult);
  const task = title ? `“${title}”` : id ? `task ${id.startsWith('#') ? id : `#${id}`}` : 'a task';
  const status = namedArgument(args, ['status']);
  if (status === 'completed') return `Marking ${task} complete`;
  if (status === 'in_progress') return `Starting ${task}`;
  if (status === 'pending') return `Returning ${task} to pending`;
  if (status === 'blocked') return `Marking ${task} blocked`;
  if (namedArgument(args, ['text'])) return `Renaming ${task}`;
  return `Updating ${task}`;
};

const summarizeExtensionManagerCall = (
  toolName: string,
  args: Record<string, unknown>
): string | null => {
  const extension = namedEntity(args, ['extension_name', 'registry_id']);

  if (toolName === 'manage_extensions') {
    const action = namedArgument(args, ['action']);
    if (action === 'enable') return extension ? `Attaching ${extension}` : 'Attaching an extension';
    if (action === 'disable')
      return extension ? `Detaching ${extension}` : 'Detaching an extension';
    return extension ? `Managing ${extension}` : 'Managing extensions';
  }
  if (toolName === 'install_extension') {
    const label = extension ? `Installing ${extension}` : 'Installing an extension';
    return args.enable === false ? `${label} without attaching it` : label;
  }
  if (toolName === 'delete_extension_package') {
    // The count comes FIRST because the Rust side merges both arguments:
    // `preflight_delete_registry_ids` does `requested.insert(0, registry_id)`,
    // so a call carrying `registry_id` AND `registry_ids` deletes all of them.
    // Naming the singular as soon as it is present would report a three-package
    // delete as one. Same shape in `removeSkillPackage` below.
    const total =
      countNamedArguments(args, 'registry_ids') + (namedArgument(args, ['registry_id']) ? 1 : 0);
    if (total > 1) return `Removing ${total} extension packages`;
    if (extension) return `Removing extension package ${extension}`;
    return total === 1 ? 'Removing 1 extension package' : 'Removing an extension package';
  }
  if (toolName === 'remove_extension') {
    // Same count-first shape as its marketplace sibling above:
    // `preflight_delete_identifiers` is literally the same function, so a call
    // carrying `extension_name` AND `extension_names` removes all of them.
    const total =
      countNamedArguments(args, 'extension_names') +
      (namedArgument(args, ['extension_name']) ? 1 : 0);
    if (total > 1) return `Removing ${total} extensions`;
    if (extension) return `Removing extension ${extension}`;
    return total === 1 ? 'Removing 1 extension' : 'Removing an extension';
  }
  // One tool now, and the summary reads off the ARGUMENT rather than the name:
  // browsing is this call with no query. The retired name stays in the match
  // because persisted transcripts still hold calls made under it.
  if (
    toolName === 'search_marketplace_extensions' ||
    toolName === 'browse_marketplace_extensions'
  ) {
    const query = namedArgument(args, ['query']);
    return query ? `Searching extensions for ${query}` : 'Browsing available extensions';
  }
  if (toolName === 'search_available_extensions')
    return 'Listing extensions available to this chat';
  return null;
};

const summarizeSkillsCall = (toolName: string, args: Record<string, unknown>): string | null => {
  const skill = namedEntity(args, ['name', 'registry_id']);

  // `setSkillEnabled` carries the verb in `enabled`; the retired pair carried it
  // in the name, and persisted transcripts still hold both.
  if (
    toolName === 'setSkillEnabled' ||
    toolName === 'hotLoadSkill' ||
    toolName === 'hotUnloadSkill'
  ) {
    const enabling = toolName === 'hotLoadSkill' || args?.enabled === true;
    if (enabling)
      return skill ? `Loading skill ${skill} into this chat` : 'Loading a skill into this chat';
    return skill ? `Unloading skill ${skill} from this chat` : 'Unloading a skill from this chat';
  }
  if (toolName === 'searchMarketplaceSkills' || toolName === 'browseMarketplaceSkills') {
    const query = namedArgument(args, ['query']);
    return query ? `Searching skills for ${query}` : 'Browsing available skills';
  }
  if (toolName === 'installMarketplaceSkill') {
    const label = skill ? `skill ${skill}` : 'a skill';
    return args.dry_run === true ? `Previewing installation of ${label}` : `Installing ${label}`;
  }
  if (toolName === 'importSkillPackage') {
    const source = namedArgument(args, ['file_path', 'url']);
    const label = source ? `skill package ${basename(source)}` : 'a skill package';
    return args.dry_run === true ? `Previewing installation of ${label}` : `Installing ${label}`;
  }
  if (toolName === 'removeSkillPackage') {
    // `preflight_removal_targets` merges `name` into `names` the same way
    // `delete_extension_package` merges its pair, so the count leads here too.
    const total = countNamedArguments(args, 'names') + (namedArgument(args, ['name']) ? 1 : 0);
    if (total > 1) return `Removing ${total} skill packages`;
    if (skill) return `Removing skill package ${skill}`;
    return total === 1 ? 'Removing 1 skill package' : 'Removing a skill package';
  }
  return null;
};

const summarizeBrowserTabs = (args: Record<string, unknown>): string => {
  const action = namedArgument(args, ['action']);
  if (action === 'list') return 'Listing browser tabs';
  if (action === 'new') return 'Opening a new browser tab';
  if (action === 'close') return 'Closing a browser tab';
  if (action === 'select') return 'Selecting a browser tab';
  return 'Managing browser tabs';
};

const summarizeSafeArguments = (args: Record<string, unknown>): string | null => {
  const hidden = /(?:content|text|body|password|secret|token|credential|api[_-]?key)/i;
  const parts = Object.entries(args)
    .filter(
      ([key, value]) => !hidden.test(key) && ['string', 'number', 'boolean'].includes(typeof value)
    )
    .slice(0, 2)
    .map(([key, value]) => `${humanize(key)}: ${compactValue(value, 40)}`);
  return parts.length > 0 ? parts.join(' · ') : null;
};

const withSafeArguments = (label: string, args: Record<string, unknown>): string => {
  const details = summarizeSafeArguments(args);
  return details ? `${label} · ${details}` : label;
};

export function summarizeToolCall(toolCall: ToolCallSummaryInput, toolResult?: unknown): string {
  const args = toolCall.arguments ?? {};
  const toolName = getToolName(toolCall.name);
  const displayName = humanize(toolName);
  const toolWords = toolName
    .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
    .toLowerCase()
    .split(/[_-]/)
    .filter(Boolean);
  const hasToolWord = (...words: string[]) => words.some((word) => toolWords.includes(word));
  const target = firstPresent(args, [
    'path',
    'file',
    'file_path',
    'filename',
    'name',
    'uri',
    'url',
    'ref_id',
  ]);
  const targetName = target ? basename(target) : null;
  const command = firstPresent(args, ['cmd', 'command', 'script']);
  const operation = firstPresent(args, ['operation', 'action', 'method']);

  // Before the generic name-matching chain: a delegation must be identifiable
  // by its task, not by the fact that it is a delegation.
  if (toolName === 'subagent') return summarizeDelegation(args);

  const todoSummary = toolCall.name.startsWith('todo__')
    ? summarizeTodoCall(toolName, args, toolResult)
    : null;
  if (todoSummary) return todoSummary;

  const extensionManagerSummary = toolCall.name.startsWith('extensionmanager__')
    ? summarizeExtensionManagerCall(toolName, args)
    : null;
  if (extensionManagerSummary) return extensionManagerSummary;

  const skillsSummary = toolCall.name.startsWith('skills__')
    ? summarizeSkillsCall(toolName, args)
    : null;
  if (skillsSummary) return skillsSummary;

  if (toolCall.name.startsWith('knowledge__')) {
    const baseId = namedArgument(args, ['kb_id']);
    const base = baseId === 'soul' ? 'Soul' : baseId;
    if (toolName === 'kb_get_active') return 'Checking the primary knowledge base';
    if (toolName === 'kb_list_bases') return 'Listing knowledge bases';
    if (toolName === 'kb_list_pages')
      return `Listing pages in ${base ?? 'the primary knowledge base'}`;
    if (toolName === 'kb_read_page') {
      return `Reading ${targetName ?? 'a knowledge page'}${base ? ` in ${base}` : ''}`;
    }
    if (toolName === 'kb_search') {
      const query = namedArgument(args, ['query']);
      return `Searching ${base ?? 'knowledge bases'}${query ? ` for ${query}` : ''}`;
    }
  }

  if (toolName === 'browser_tabs') return summarizeBrowserTabs(args);

  // code_execution's module tools and the skills loader carry their targets
  // under argument names (`module_path`, `terms`, `name`) the generic chains
  // don't know, so their labels degraded to the opaque "Read Module" /
  // "Search Modules" verbs the transcript audit flagged (#27). Name the exact
  // target in the collapsed row. Matched on the FULL prefixed name (the
  // built-in extensions register as `code_execution` / `skills`), so an
  // unrelated extension's same-named tool keeps its own generic label.
  if (toolCall.name === 'code_execution__read_module') {
    const modulePath = compactValue(args.module_path ?? '');
    return modulePath ? `Reading module ${modulePath}` : 'Reading a module';
  }
  if (toolCall.name === 'code_execution__search_modules') {
    const terms = summarizeSearchQuery(args.terms);
    return terms ? `Searching modules for ${terms}` : 'Searching modules';
  }
  if (toolCall.name === 'skills__loadSkill') {
    const skillName = compactValue(args.name ?? '');
    return skillName ? `Loading skill ${skillName}` : 'Loading a skill';
  }

  if (toolName === 'text_editor' || hasToolWord('editor')) {
    if (args.command === 'view' && targetName) return `Reading ${targetName}`;
    if (args.command === 'write' && targetName) return `Writing ${targetName}`;
    if (args.command === 'str_replace' && targetName) return `Editing ${targetName}`;
    if (targetName) return `Updating ${targetName}`;
  }

  if (toolName === 'shell' || toolName === 'exec_command' || hasToolWord('command')) {
    return command ? `Running ${compactValue(command)}` : `Running a command`;
  }

  if (toolName === 'apply_patch') return `Applying a code patch`;
  if (toolName === 'view_image')
    return targetName ? `Inspecting ${targetName}` : `Inspecting an image`;
  if (hasToolWord('screenshot')) return `Capturing a screenshot`;
  if (toolName.includes('imagegen')) return `Generating an image`;

  if (hasToolWord('read')) {
    return targetName
      ? `Reading ${targetName}`
      : withSafeArguments(`Reading ${displayName.replace(/^Read /, '')}`, args);
  }

  if (hasToolWord('open')) {
    const subject = displayName.replace(/^Open /, '').replace(/^In /, 'in ');
    return targetName ? `Opening ${targetName}` : withSafeArguments(`Opening ${subject}`, args);
  }

  if (hasToolWord('fetch')) {
    return targetName
      ? `Fetching ${targetName}`
      : withSafeArguments(`Fetching ${displayName.replace(/^Fetch /, '')}`, args);
  }

  if (hasToolWord('write', 'update', 'edit')) {
    return targetName
      ? `Updating ${targetName}`
      : withSafeArguments(`Updating ${displayName.replace(/^(Write|Update|Edit) /, '')}`, args);
  }

  if (hasToolWord('create')) {
    return targetName
      ? `Creating ${targetName}`
      : withSafeArguments(`Creating ${displayName.replace(/^Create /, '')}`, args);
  }

  if (hasToolWord('delete', 'remove')) {
    return targetName
      ? `Removing ${targetName}`
      : withSafeArguments(`Removing ${displayName.replace(/^(Delete|Remove) /, '')}`, args);
  }

  if (hasToolWord('list')) {
    return targetName
      ? `Listing ${targetName}`
      : withSafeArguments(`Listing ${displayName.replace(/^List /, '')}`, args);
  }

  if (hasToolWord('search', 'query')) {
    const query = summarizeSearchQuery(
      firstPresent(args, ['q', 'query', 'search_query', 'image_query', 'pattern', 'name'])
    );
    return query
      ? `Searching for ${query}`
      : withSafeArguments(`Searching ${displayName.replace(/^(Search|Query) /, '')}`, args);
  }

  if (hasToolWord('download')) {
    return targetName
      ? `Downloading ${targetName}`
      : withSafeArguments(`Downloading ${displayName.replace(/^Download /, '')}`, args);
  }

  if (hasToolWord('send', 'post')) {
    return targetName ? `Sending to ${targetName}` : `${displayName}`;
  }

  if (toolName === 'sheets_tool' || hasToolWord('sheet', 'sheets', 'spreadsheet')) {
    return operation
      ? `${humanize(compactValue(operation))} in a spreadsheet`
      : withSafeArguments(displayName, args);
  }

  if (toolName === 'docs_tool' || hasToolWord('doc', 'docs', 'document', 'documents')) {
    return operation
      ? `${humanize(compactValue(operation))} in a document`
      : withSafeArguments(displayName, args);
  }

  if (toolName === 'execute_code') {
    const toolGraph = normalizeToolGraph(args.tool_graph);
    if (toolGraph.length > 0) return summarizeToolGraph(toolGraph);
    return `Executing code`;
  }

  if (targetName) return `${displayName} for ${targetName}`;

  const safeArguments = summarizeSafeArguments(args);
  if (safeArguments) return `${displayName} · ${safeArguments}`;

  return displayName;
}

export const logToString = (logMessage: NotificationEvent) => {
  const message = logMessage.message as { method: string; params: unknown };
  const params = message.params as Record<string, unknown>;

  // Special case for the developer system shell logs
  if (
    params &&
    params.data &&
    typeof params.data === 'object' &&
    'output' in params.data &&
    'stream' in params.data
  ) {
    return `[${params.data.stream}] ${params.data.output}`;
  }

  // Issue #72: the developer shell's foreground heartbeat. A command that prints
  // nothing while it works — a big `find`, a slow query — used to leave the card
  // saying "Working through the tool call" indefinitely, which is
  // indistinguishable from a hung agent. Render its ready-made sentence rather
  // than the raw JSON the fallback below would produce.
  if (
    params &&
    params.data &&
    typeof params.data === 'object' &&
    'message' in params.data &&
    typeof (params.data as { message: unknown }).message === 'string'
  ) {
    return (params.data as { message: string }).message;
  }

  return typeof params.data === 'string' ? params.data : JSON.stringify(params.data);
};

const notificationToProgress = (notification: NotificationEvent): Progress => {
  const message = notification.message as { method: string; params: unknown };
  return message.params as Progress;
};

// Helper function to extract toolcall name
const getToolName = (toolCallName: string): string => {
  const lastIndex = toolCallName.lastIndexOf('__');
  if (lastIndex === -1) return toolCallName;

  return toolCallName.substring(lastIndex + 2);
};

function ToolCallView({
  isCancelledMessage,
  sessionId,
  toolCall,
  toolResponse,
  notifications,
  turnActive = false,
  providerExecution = null,
  onOpenArtifact,
  workingDir,
}: ToolCallViewProps) {
  const [responseStyle, setResponseStyle] = useState(() => localStorage.getItem('response_style'));

  useEffect(() => {
    const handleStorageChange = () => {
      setResponseStyle(localStorage.getItem('response_style'));
    };

    window.addEventListener('storage', handleStorageChange);

    window.addEventListener('responseStyleChanged', handleStorageChange);

    return () => {
      window.removeEventListener('storage', handleStorageChange);
      window.removeEventListener('responseStyleChanged', handleStorageChange);
    };
  }, []);

  const isToolDetails = toolCall?.arguments && Object.entries(toolCall.arguments).length > 0;

  const toolError = getToolResultError(toolResponse?.toolResult);

  // Issue #56, DR-26 / Task 57. A cross-institutional refusal Gate C would clear
  // on a grant — the ONE refusal in this feature whose way out is a thing the
  // user does rather than a model they switch to. `null` for every other
  // failure, including the two sites that compose the same refusal without
  // consulting a grant.
  //
  // ⚠ **The tool-output guardrail does not touch this text, and the marker
  // parse below never needed repairing.** Recorded because the commit that
  // added the frame-stripping claimed otherwise. `cross_affiliation_refusal`
  // returns an `ErrorData`, so the refusal arrives on `toolResult.status ===
  // 'error'`, and `guard_tool_result` frames only the `Ok` branch. It reaches
  // `displayError` unframed, exactly as before. What IS framed is an MCP
  // result carrying `isError: true`, a different path through the same
  // function — that one is unwrapped, and it is not where this marker lives.
  //
  // ⚠ **Gated on the session as well as on the text, so ONE expression decides
  // both the card and the disclosure below.** A grant is keyed on a chat, and
  // the saved-transcript surface (`sessions/SessionViewComponents.tsx`) passes
  // none — so an offer there is an offer of nothing. Reading the session here
  // rather than only inside the card is what stops the two from drifting into
  // "expanded for a control that does not render".
  const acceptOffer = sessionId ? crossAffiliationOffer(toolError) : null;

  // Executed sub-call telemetry (#28): what a coordinated execute_code step
  // actually ran, with exact inputs and per-call status. Shown alongside the
  // declared tool_graph plan — never force-matched to it, since loops and
  // conditionals mean plan and execution can legitimately differ.
  const resultMeta = (toolResponse?.toolResult as ToolResultWithMeta | undefined)?.value?._meta;
  const executedCalls = normalizeExecutedCalls(resultMeta?.['biorouter/tool-calls']);
  const droppedCallCount = resultMeta?.['biorouter/tool-calls-dropped'];
  const droppedExecutedCalls =
    typeof droppedCallCount === 'number' &&
    Number.isSafeInteger(droppedCallCount) &&
    droppedCallCount >= 0
      ? droppedCallCount
      : 0;

  // Status is derived from FACTS — is there a result, and is the turn still
  // running — never from "am I the last message". The old derivation keyed off
  // `!isStreamingMessage`, which flips false for a still-executing tool call the
  // instant any later message arrives, painting a pending card green "Finished".
  //
  // The response-less-after-the-turn-ended case still must not spin forever
  // (some backends never emit a tool response: cancelled turns, BioRouterMode
  // ::Chat skips, crashed extensions), so it routes to a distinct `interrupted`
  // state rather than a fabricated success.
  const loadingStatus: ToolLoadingStatus = toolResponse
    ? toolError
      ? 'error'
      : 'success'
    : turnActive
      ? 'loading'
      : 'interrupted';

  const toolResults =
    loadingStatus === 'success' && toolResponse?.toolResult
      ? getToolResultContent(toolResponse.toolResult)
      : [];

  const logs = notifications
    ?.filter((notification) => {
      const message = notification.message as { method?: string };
      return message.method === 'notifications/message';
    })
    .map(logToString);

  const progress = notifications
    ?.filter((notification) => {
      const message = notification.message as { method?: string };
      return message.method === 'notifications/progress';
    })
    .map(notificationToProgress)
    .reduce((map, item) => {
      const key = item.progressToken;
      if (!map.has(key)) {
        map.set(key, []);
      }
      map.get(key)!.push(item);
      return map;
    }, new Map<string, Progress[]>());

  const progressEntries = [...(progress?.values() || [])].map(
    (entries) => entries.sort((a, b) => b.progress - a.progress)[0]
  );

  // Map ToolLoadingStatus to ToolCallStatus
  const getToolCallStatus = (loadingStatus: ToolLoadingStatus): ToolCallStatus => {
    switch (loadingStatus) {
      case 'success':
        return 'success';
      case 'error':
        return 'error';
      case 'loading':
        return 'loading';
      default:
        return 'pending';
    }
  };

  const toolCallStatus = getToolCallStatus(loadingStatus);
  const toolSummary = summarizeToolCall(toolCall, toolResponse?.toolResult);
  const latestProgress = progressEntries[0]?.message;
  const latestLog = logs && logs.length > 0 ? logs[logs.length - 1] : undefined;
  const liveDetail =
    loadingStatus === 'loading'
      ? compactValue(latestProgress || latestLog || 'Working through the tool call')
      : loadingStatus === 'error'
        ? 'Tool call failed'
        : loadingStatus === 'interrupted'
          ? 'No result'
          : toolResults.length > 0
            ? `${toolResults.length} result${toolResults.length === 1 ? '' : 's'} ready`
            : null;

  const toolLabel = (
    <span className="flex items-center gap-2 min-w-0 text-left leading-6">
      <ToolIconWithStatus
        ToolIcon={getToolCallIcon(toolCall.name)}
        status={toolCallStatus}
        className="mt-px"
      />
      {/* The summary truncates; the status suffix does not.
          Both used to live inside ONE `truncate` span, so a single ellipsis
          budget covered them and the SUFFIX was what got eaten — consecutive
          rows read "· 1 …", "· 1 result rea…", and where the command was
          longest, nothing at all while the chevron survived. The suffix is the
          shorter and more valuable of the two (it says whether there is
          anything to open), so it is the part that must never be cut. Making
          this a flex row with a `min-w-0` truncating summary and a `shrink-0`
          suffix is the fix; a wider clipper would only move the threshold. */}
      <span className="flex min-w-0 flex-1 items-baseline text-text-muted">
        <span className="min-w-0 truncate">
          {loadingStatus === 'loading'
            ? 'Working on'
            : loadingStatus === 'error'
              ? 'Problem with'
              : loadingStatus === 'interrupted'
                ? 'Stopped'
                : 'Ran'}{' '}
          <span className="text-text-default">{toolSummary}</span>
        </span>
        {liveDetail && (
          <span
            className={cn(
              'shrink-0 whitespace-nowrap pl-1 text-text-muted/70',
              loadingStatus === 'loading' && 'animate-pulse'
            )}
          >
            · {liveDetail}
          </span>
        )}
        {/* The ONE deliberate visual difference in the whole mirror feature. A
            `child` call ran in the coding agent's own sandbox and passed none of
            Biorouter's gates, and a person reading a command row is entitled to
            know that. Quiet by design — muted text in the row's own type, no
            badge, no colour, no outline (D-17: a tool call is a line, not a
            card) — because it states a provenance, not a failure. `bridged`
            passed every gate an API provider's call passes, so it renders with
            nothing here at all. */}
        {providerExecution === 'child' && (
          <span
            className="shrink-0 whitespace-nowrap pl-1 text-text-muted/70"
            title={CHILD_EXECUTED_TITLE}
          >
            · not gated by Biorouter
          </span>
        )}
      </span>
    </span>
  );
  return (
    // ⚠ A grantable cross-institutional refusal starts EXPANDED (issue #56, Task
    // 57). A failed tool call is otherwise a collapsed line, and a way out that
    // only exists behind a disclosure most people never open is the hard block
    // this task exists to remove, one click further away. Every other failure
    // keeps the quiet default — including this same refusal replayed in a saved
    // transcript, where `acceptOffer` is null because there is no chat to accept
    // against and the disclosure would open onto nothing.
    <ToolCallExpandable
      isStartExpanded={acceptOffer !== null}
      isForceExpand={false}
      label={toolLabel}
    >
      {(() => {
        const toolName = toolCall.name.substring(toolCall.name.lastIndexOf('__') + 2);
        const toolGraph = normalizeToolGraph(toolCall.arguments?.tool_graph);
        const code = toolCall.arguments?.code as unknown as string | undefined;
        const hasToolGraph = toolName === 'execute_code' && toolGraph.length > 0;

        if (hasToolGraph) {
          return (
            <div className="border-t border-border-subtle">
              <ToolGraphView toolGraph={toolGraph} code={code} />
            </div>
          );
        }

        if (isToolDetails) {
          return (
            <div className="border-t border-border-subtle">
              <ToolDetailsView toolCall={toolCall} isStartExpanded={false} />
            </div>
          );
        }

        return null;
      })()}

      {(executedCalls.length > 0 || droppedExecutedCalls > 0) && (
        <div className="border-t border-border-subtle">
          <ExecutedCallsView calls={executedCalls} dropped={droppedExecutedCalls} />
        </div>
      )}

      {toolError && (
        <div className="border-t border-border-subtle p-3">
          <NotificationContent status="error" title="Tool call failed" message={toolError} />
          {/*
            Issue #56, DR-26 / Task 57 — the accept control, on the surface the
            refusal lands on and directly under the daemon's own words. DR-26
            rules warn-then-allow-if-the-user-insists; without this the refusal
            arrived, the model was told to ask, and the person at the keyboard
            had nothing to press.
          */}
          {acceptOffer && (
            <CrossAffiliationAcceptCard sessionId={sessionId} extension={acceptOffer.extension} />
          )}
        </div>
      )}

      {logs && logs.length > 0 && (
        <div className="border-t border-border-subtle">
          <ToolLogsView
            logs={logs}
            working={loadingStatus === 'loading'}
            isStartExpanded={
              loadingStatus === 'loading' || responseStyle === 'detailed' || responseStyle === null
            }
          />
        </div>
      )}

      {toolResults.length === 0 &&
        progressEntries.length > 0 &&
        progressEntries.map((entry, index) => (
          <div className="p-3 border-t border-border-subtle" key={index}>
            <ProgressBar progress={entry.progress} total={entry.total} message={entry.message} />
          </div>
        ))}

      {/* Tool Output */}
      {!isCancelledMessage && (
        <>
          {toolResults.map((result, index) => (
            <div key={index} className={cn('border-t border-border-subtle')}>
              <ToolResultView
                result={result}
                isStartExpanded={false}
                onOpenArtifact={onOpenArtifact}
                workingDir={workingDir}
              />
            </div>
          ))}
        </>
      )}
    </ToolCallExpandable>
  );
}

interface ToolDetailsViewProps {
  toolCall: {
    name: string;
    arguments: Record<string, unknown>;
  };
  isStartExpanded: boolean;
}

function ToolDetailsView({ toolCall, isStartExpanded }: ToolDetailsViewProps) {
  return (
    <ToolCallExpandable
      label={<span className={TOOL_DISCLOSURE_LABEL_CLASS}>View tool details</span>}
      isStartExpanded={isStartExpanded}
    >
      <div className={TOOL_INTERIOR_CLASS}>
        {toolCall.arguments && (
          <ToolCallArguments args={toolCall.arguments as Record<string, ToolCallArgumentValue>} />
        )}
      </div>
    </ToolCallExpandable>
  );
}

interface ToolGraphViewProps {
  toolGraph: ToolGraphNode[];
  code?: string;
}

function ToolGraphView({ toolGraph, code }: ToolGraphViewProps) {
  // The one shared palette (styles/codeTheme.ts) — the same source chat code
  // blocks and the artifact panel read, so the generated-code view can never
  // drift from them. Both hooks are non-throwing outside a provider.
  const codeStyle = codeThemesByFamily[useThemeFamily()][useResolvedTheme()];

  return (
    <div className={TOOL_INTERIOR_CLASS}>
      <ol className="overflow-x-auto whitespace-pre-wrap text-secondary text-text-muted">
        {toolGraph.map((node, index) => {
          const dependencies = node.depends_on.map((dependency) => dependency + 1).join(', ');
          return (
            <li key={index} title={`Tool: ${node.tool}`}>
              {index + 1}. {toolGraphNodeLabel(node)}
              {dependencies && ` (uses ${dependencies})`}
            </li>
          );
        })}
      </ol>
      {code && (
        <div className="-mx-3 mt-2 border-t border-border-subtle">
          <ToolCallExpandable
            label={<span className={TOOL_DISCLOSURE_LABEL_CLASS}>View generated code</span>}
            isStartExpanded={false}
          >
            {/* bg-background-code: the ground the syntax palette is verified
                against (see MarkdownContent's CodeBlock). No line numbers, so
                long lines may wrap (never combine wrapping with line numbers
                in react-syntax-highlighter). */}
            <div className="w-full overflow-x-auto bg-background-code">
              <SyntaxHighlighter
                style={codeStyle}
                language="javascript"
                PreTag="div"
                customStyle={{
                  margin: 0,
                  padding: '12px',
                  background: 'transparent',
                  width: '100%',
                  maxWidth: '100%',
                }}
                codeTagProps={{
                  style: {
                    whiteSpace: 'pre-wrap',
                    wordBreak: 'break-word',
                    overflowWrap: 'break-word',
                    fontFamily: CODE_FONT_FAMILY,
                    fontSize: CODE_FONT_SIZE,
                    lineHeight: CODE_LINE_HEIGHT,
                  },
                }}
                showLineNumbers={false}
                wrapLines={false}
              >
                {code}
              </SyntaxHighlighter>
            </div>
          </ToolCallExpandable>
        </div>
      )}
    </div>
  );
}

/** Parse a recorded args JSON string; null if truncated/malformed. */
function parsedCallArguments(args?: string): Record<string, ToolCallArgumentValue> | null {
  if (!args) return null;
  try {
    const parsed: unknown = JSON.parse(args);
    const record = recordOf(parsed);
    return record ? (record as Record<string, ToolCallArgumentValue>) : null;
  } catch {
    return null;
  }
}

/**
 * The EXECUTED sub-calls of a coordinated step (#28): which tools actually
 * ran, with which exact inputs, and which one failed — so a failed step is
 * attributable to a specific call instead of reading as "the step failed".
 */
function ExecutedCallsView({ calls, dropped }: { calls: ExecutedToolCall[]; dropped: number }) {
  const total = calls.length + dropped;
  return (
    <ToolCallExpandable
      label={
        <span className={TOOL_DISCLOSURE_LABEL_CLASS}>
          {dropped > 0
            ? `View recorded calls (${calls.length} of ${total} executed)`
            : `View executed calls (${calls.length})`}
        </span>
      }
      isStartExpanded={false}
    >
      <div className={TOOL_INTERIOR_CLASS}>
        {calls.map((call, index) => (
          <ExecutedCallRow key={index} call={call} />
        ))}
        {dropped > 0 && (
          <div className="py-1 text-supporting text-text-muted">
            {dropped} executed call{dropped === 1 ? ' was' : 's were'} not recorded, so{' '}
            {dropped === 1 ? 'its' : 'their'} details are unavailable.
          </div>
        )}
      </div>
    </ToolCallExpandable>
  );
}

/**
 * Executed-call telemetry is untrusted wire data replayed from the result
 * meta, so keys and values render as PLAIN TEXT only — never through
 * `ToolCallArguments`, whose expanded strings go through `MarkdownContent`
 * and would turn crafted arguments into live links or remote-image fetches.
 */
function ExecutedCallArguments({ args }: { args: Record<string, ToolCallArgumentValue> }) {
  return (
    <div className="my-2">
      {Object.entries(args).map(([key, value]) => (
        <div key={key} className="mb-2 flex flex-row text-secondary">
          <span className="min-w-[140px] shrink-0 text-text-muted">{key}</span>
          <pre className="min-w-0 max-w-full flex-1 overflow-x-auto whitespace-pre-wrap break-words font-mono text-code text-text-muted">
            {typeof value === 'string' ? value : JSON.stringify(value, null, 2)}
          </pre>
        </div>
      ))}
    </div>
  );
}

function ExecutedCallRow({ call }: { call: ExecutedToolCall }) {
  const parsedArgs = parsedCallArguments(call.args);
  const taskSummary = call.todoTask
    ? summarizeTodoCall(
        'todo_update',
        parsedArgs ?? {},
        undefined,
        compactExecutedTaskTitle(call.todoTask.title)
      )
    : null;
  const summary =
    taskSummary ?? summarizeToolCall({ name: call.tool, arguments: parsedArgs ?? {} });
  return (
    <ToolCallExpandable
      label={
        <span className="flex min-w-0 items-center gap-2 text-left leading-6">
          <ToolIconWithStatus
            ToolIcon={getToolCallIcon(call.tool)}
            status={call.status === 'error' ? 'error' : 'success'}
            className="mt-px"
          />
          <span className="min-w-0 flex-1 truncate text-text-muted">
            <span className="text-text-default">{summary}</span>
            {call.status === 'error' && ' · failed'}
          </span>
        </span>
      }
      isStartExpanded={false}
    >
      <div className={TOOL_INTERIOR_CLASS}>
        {parsedArgs && Object.keys(parsedArgs).length > 0 ? (
          <ExecutedCallArguments args={parsedArgs} />
        ) : call.args ? (
          // Truncated/malformed JSON still shows the exact recorded text.
          <pre className="whitespace-pre-wrap break-all font-mono text-code text-text-muted">
            {call.args}
          </pre>
        ) : (
          <div className="text-supporting text-text-muted">No arguments recorded.</div>
        )}
        {call.error && (
          <div className="mt-2">
            <NotificationContent status="error" title={`${summary} failed`} message={call.error} />
          </div>
        )}
      </div>
    </ToolCallExpandable>
  );
}

interface ToolResultViewProps {
  result: Content;
  isStartExpanded: boolean;
  onOpenArtifact: (artifact: ArtifactSource) => void;
  workingDir?: string;
}

function ToolResultView({
  result,
  isStartExpanded,
  onOpenArtifact,
  workingDir,
}: ToolResultViewProps) {
  const hasText = (c: Content): c is Content & { text: string } =>
    'text' in c && typeof (c as Record<string, unknown>).text === 'string';

  const hasImage = (c: Content): c is Content & { data: string; mimeType: string } => {
    if (!('data' in c && 'mimeType' in c)) return false;
    const mimeType = (c as Record<string, unknown>).mimeType;
    return typeof mimeType === 'string' && mimeType.startsWith('image');
  };

  const hasResource = (c: Content): c is Content & { resource: unknown } => 'resource' in c;

  return (
    <ToolCallExpandable
      label={<span className={TOOL_DISCLOSURE_LABEL_CLASS}>View output</span>}
      isStartExpanded={isStartExpanded}
    >
      <div className={TOOL_INTERIOR_CLASS}>
        {hasText(result) && (
          <MarkdownContent
            content={result.text}
            className="whitespace-pre-wrap max-w-full overflow-x-auto"
            onOpenArtifact={onOpenArtifact}
            workingDir={workingDir}
          />
        )}
        {hasImage(result) && (
          <img
            src={`data:${result.mimeType};base64,${result.data}`}
            alt="Tool result"
            className="max-w-full h-auto rounded-md my-2"
            onError={(e) => {
              console.error('Failed to load image');
              e.currentTarget.style.display = 'none';
            }}
          />
        )}
        {hasResource(result) && (
          // ⚠ NOT `font-sans`. This is `JSON.stringify(…, null, 2)` in a <pre>
          // — the same value class the other two raw dumps in this file render
          // in `font-mono text-code` (ExecutedCallArguments above, and the
          // malformed-args fallback beside it). All three are disclosures of
          // ONE tool call, so expanding "View output" and "View executed calls"
          // put pretty-printed JSON on screen in two typefaces at once.
          // A proportional face also defeats the point of the <pre>: the
          // two-space indent `stringify` emits only reads as structure when the
          // glyphs are fixed-width. D-31 in styles/main.css: mono earns code.
          <pre className="font-mono text-code whitespace-pre-wrap break-all overflow-x-auto max-w-full">
            {JSON.stringify(result, null, 2)}
          </pre>
        )}
      </div>
    </ToolCallExpandable>
  );
}

function ToolLogsView({
  logs,
  working,
  isStartExpanded,
}: {
  logs: string[];
  working: boolean;
  isStartExpanded?: boolean;
}) {
  const boxRef = useRef<HTMLDivElement>(null);

  // Whenever logs update, jump to the newest entry
  useEffect(() => {
    if (boxRef.current) {
      boxRef.current.scrollTop = boxRef.current.scrollHeight;
    }
  }, [logs.length]);
  // normally we do not want to put .length on an array in react deps:
  //
  // if the objects inside the array change but length doesn't change you want updates
  //
  // in this case, this is array of strings which once added do not change so this cuts
  // down on the possibility of unwanted runs

  return (
    <ToolCallExpandable
      label={
        <span className={cn(TOOL_DISCLOSURE_LABEL_CLASS, 'flex items-center')}>
          <span>View live logs</span>
          {working && (
            <div className="mx-2 inline-block">
              <span
                className="inline-block animate-spin rounded-full border-2 border-t-transparent border-current"
                style={{ width: 8, height: 8 }}
                role="status"
                aria-label="Loading spinner"
              />
            </div>
          )}
        </span>
      }
      isStartExpanded={isStartExpanded}
    >
      <div
        ref={boxRef}
        className={cn(
          TOOL_INTERIOR_CLASS,
          'flex flex-col items-start space-y-2 overflow-y-auto',
          working ? 'max-h-[4rem]' : 'max-h-[20rem]'
        )}
      >
        {logs.map((log, i) => (
          <span
            key={i}
            className="w-full whitespace-pre-wrap break-words font-mono text-code text-text-muted"
          >
            {log}
          </span>
        ))}
      </div>
    </ToolCallExpandable>
  );
}

const ProgressBar = ({ progress, total, message }: Omit<Progress, 'progressToken'>) => {
  const isDeterminate = typeof total === 'number';
  const percent = isDeterminate ? Math.min((progress / total!) * 100, 100) : 0;

  return (
    <div className="w-full space-y-2">
      {message && <div className="font-sans text-sm text-text-muted">{message}</div>}

      <div className="w-full bg-background-muted rounded-md h-4 overflow-hidden relative">
        {isDeterminate ? (
          <div
            className="bg-background-accent h-full w-full origin-left transition-transform duration-[var(--motion-base)] ease-[var(--ease-out)]"
            style={{ transform: `scaleX(${Math.min(Math.max(percent / 100, 0), 1)})` }}
          />
        ) : (
          <div className="absolute inset-0 animate-pulse bg-background-accent" />
        )}
      </div>
    </div>
  );
};
