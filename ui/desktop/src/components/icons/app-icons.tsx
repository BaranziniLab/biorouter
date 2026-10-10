/**
 * App icon library — Lucide React (ISC, lucide.dev)
 * All icons render at strokeWidth=1.5 for a consistent light/outline appearance.
 */

import React, { useId } from 'react';
import {
  Activity as _Activity,
  AlertCircle as _AlertCircle,
  AlertTriangle as _AlertTriangle,
  AlignLeft as _AlignLeft,
  AppWindow as _AppWindow,
  AppWindowMac as _AppWindowMac,
  Archive as _Archive,
  ArrowDown as _ArrowDown,
  ArrowLeft as _ArrowLeft,
  ArrowRight as _ArrowRight,
  ArrowUp as _ArrowUp,
  BookMarked as _BookMarked,
  BookOpen as _BookOpen,
  Bookmark as _Bookmark,
  BookmarkPlus as _BookmarkPlus,
  Bot as _Bot,
  Brain as _Brain,
  Bug as _Bug,
  Calendar as _Calendar,
  Camera as _Camera,
  ChartColumn as _ChartColumn,
  Check as _Check,
  CheckCircle as _CheckCircle,
  CheckCircle2 as _CheckCircle2,
  ChevronDown as _ChevronDown,
  ChevronLeft as _ChevronLeft,
  ChevronRight as _ChevronRight,
  ChevronUp as _ChevronUp,
  ChevronsDownUp as _ChevronsDownUp,
  Circle as _Circle,
  CircleDotDashed as _CircleDotDashed,
  CircleHelp as _CircleHelp,
  Clipboard as _Clipboard,
  ClipboardList as _ClipboardList,
  Clock as _Clock,
  Code as _Code,
  Copy as _Copy,
  Database as _Database,
  Download as _Download,
  Edit as _Edit,
  Edit2 as _Edit2,
  ExternalLink as _ExternalLink,
  Eye as _Eye,
  EyeOff as _EyeOff,
  File as _File,
  FileCode2 as _FileCode2,
  FilePen as _FilePen,
  FilePlus as _FilePlus,
  FileSpreadsheet as _FileSpreadsheet,
  FileStack as _FileStack,
  FileText as _FileText,
  FileX as _FileX,
  Filter as _Filter,
  Fingerprint as _Fingerprint,
  Flag as _Flag,
  FlaskConical as _FlaskConical,
  Folder as _Folder,
  FolderDot as _FolderDot,
  FolderInput as _FolderInput,
  FolderKey as _FolderKey,
  FolderOpen as _FolderOpen,
  FolderPlus as _FolderPlus,
  FolderTree as _FolderTree,
  Gauge as _Gauge,
  GitBranch as _GitBranch,
  Github as _Github,
  Globe as _Globe,
  GripVertical as _GripVertical,
  Hash as _Hash,
  HeartPulse as _HeartPulse,
  History as _History,
  Home as _Home,
  Image as _Image,
  Inbox as _Inbox,
  Info as _Info,
  KeyRound as _KeyRound,
  Landmark as _Landmark,
  Laptop as _Laptop,
  Layers as _Layers,
  Link as _Link,
  ListTodo as _ListTodo,
  Loader2 as _Loader2,
  LoaderCircle as _LoaderCircle,
  Lock as _Lock,
  LogOut as _LogOut,
  Maximize2 as _Maximize2,
  MessageSquare as _MessageSquare,
  MessageSquarePlus as _MessageSquarePlus,
  MessageSquareText as _MessageSquareText,
  MessagesSquare as _MessagesSquare,
  Monitor as _Monitor,
  Moon as _Moon,
  MoreHorizontal as _MoreHorizontal,
  Music as _Music,
  Package as _Package,
  Palette as _Palette,
  PanelLeftIcon as _PanelLeftIcon,
  PanelRight as _PanelRight,
  Paperclip as _Paperclip,
  Pause as _Pause,
  PauseCircle as _PauseCircle,
  Pencil as _Pencil,
  PictureInPicture2 as _PictureInPicture2,
  Pill as _Pill,
  Play as _Play,
  Plus as _Plus,
  Puzzle as _Puzzle,
  QrCode as _QrCode,
  RefreshCw as _RefreshCw,
  Rocket as _Rocket,
  RotateCcw as _RotateCcw,
  Save as _Save,
  ScrollText as _ScrollText,
  Search as _Search,
  SearchCode as _SearchCode,
  Send as _Send,
  Server as _Server,
  Settings as _Settings,
  Share2 as _Share2,
  Sliders as _Sliders,
  SlidersHorizontal as _SlidersHorizontal,
  Sparkles as _Sparkles,
  Square as _Square,
  SquarePen as _SquarePen,
  SquareSlash as _SquareSlash,
  StopCircle as _StopCircle,
  Sun as _Sun,
  Target as _Target,
  Terminal as _Terminal,
  TextQuote as _TextQuote,
  Tornado as _Tornado,
  Trash2 as _Trash2,
  Upload as _Upload,
  UserPlus as _UserPlus,
  Users as _Users,
  Video as _Video,
  Workflow as _Workflow,
  Wrench as _Wrench,
  X as _X,
  Zap as _Zap,
  type LucideIcon,
  type LucideProps,
} from 'lucide-react';

// ---------------------------------------------------------------------------
// Light wrapper — enforces the canonical icon contract (design.md §3.9):
// every icon renders at strokeWidth=1.5 and is monochrome via `currentColor`.
// Both are pinned *after* the prop spread so a caller can never reintroduce a
// second stroke weight (DR-53) or a hardcoded fill. `currentColor` still lets
// callers tint an icon through CSS `color` (className/style) — it only blocks a
// hex being injected via the `color` prop.
// ---------------------------------------------------------------------------
const light = (Icon: LucideIcon): React.FC<LucideProps> => {
  const Wrapped: React.FC<LucideProps> = (props) => (
    <Icon {...props} strokeWidth={1.5} color="currentColor" />
  );
  // Name the wrapper so React DevTools (and react/display-name) can identify it.
  Wrapped.displayName = `light(${Icon.displayName ?? Icon.name ?? 'Icon'})`;
  return Wrapped;
};

// ---------------------------------------------------------------------------
// Named exports — same names as the original file so no consumer changes.
// ---------------------------------------------------------------------------

export const Activity = light(_Activity);
export const AlertCircle = light(_AlertCircle);
export const Info = light(_Info);
export const AlertTriangle = light(_AlertTriangle);
// The session-review mark the cohesion spec draws (three descending rules).
export const AlignLeft = light(_AlignLeft);
export const AppWindow = light(_AppWindow);
export const AppWindowMac = light(_AppWindowMac);
export const Archive = light(_Archive);
export const ArrowDown = light(_ArrowDown);
export const ArrowLeft = light(_ArrowLeft);
/** Go on: Resume a saved chat (was Sparkles, which also meant ingest and an
 * empty graph). */
export const ArrowRight = light(_ArrowRight);
export const ArrowUp = light(_ArrowUp);
export const BookMarked = light(_BookMarked);
export const BookOpen = light(_BookOpen);
export const Bookmark = light(_Bookmark);
export const BookmarkPlus = light(_BookmarkPlus);
export const Bot = light(_Bot);
export const Brain = light(_Brain);
/** `platform__report_bug` rows. */
export const Bug = light(_Bug);
export const Calendar = light(_Calendar);
export const Camera = light(_Camera);
/** A figure: Auto Visualiser tool rows and `ui://` figure tabs. */
export const ChartColumn = light(_ChartColumn);
export const Check = light(_Check);
export const CheckIcon = Check;
export const CheckCircle = light(_CheckCircle);
export const CheckCircle2 = light(_CheckCircle2);
export const ChevronUp = light(_ChevronUp);
export const ChevronDown = light(_ChevronDown);
export const ChevronDownIcon = ChevronDown;
export const ChevronRight = light(_ChevronRight);
export const ChevronRightIcon = ChevronRight;
export const ChevronLeft = light(_ChevronLeft);
export const ChevronsDownUp = light(_ChevronsDownUp);
export const CircleIcon = light(_Circle);
export const CircleDotDashed = light(_CircleDotDashed);
/** The privacy axes' "unstated" mark — see `Landmark` below for the trio. */
export const CircleHelp = light(_CircleHelp);
export const CircleHelpIcon = CircleHelp;
export const Clipboard = light(_Clipboard);
export const ClipboardList = light(_ClipboardList);
export const Clock = light(_Clock);
export const Code = light(_Code);
export const CodeAnalysis = light(_SearchCode);
export const Copy = light(_Copy);
export const Database = light(_Database);
export const Download = light(_Download);
export const Edit = light(_Edit);
export const Edit2 = light(_Edit2);
export const ExternalLink = light(_ExternalLink);
export const Eye = light(_Eye);
export const EyeOff = light(_EyeOff);
export const File = light(_File);
export const FileCode2 = light(_FileCode2);
/** Writing or editing a file (tool rows). Reading one is `FileText`. */
export const FilePen = light(_FilePen);
export const FilePlus = light(_FilePlus);
export const FileSpreadsheet = light(_FileSpreadsheet);
export const FileStack = light(_FileStack);
export const FileText = light(_FileText);
export const FileX = light(_FileX);
/** A key fingerprint a person compares out of band before trusting it (Crew's
 * host-key and join screens). Not a sign-in or a biometric. */
export const Fingerprint = light(_Fingerprint);
export const FlaskConical = light(_FlaskConical);
export const Folder = light(_Folder);
export const FolderDot = light(_FolderDot);
export const FolderInput = light(_FolderInput);
export const FolderKey = light(_FolderKey);
export const FolderOpen = light(_FolderOpen);
export const FolderPlus = light(_FolderPlus);
export const FolderTree = light(_FolderTree);
// The Knowledge section's two `EmptyState` icons (ui-spec §4.12). Every other
// icon that section names was already re-exported here; these two were the gap,
// and a call site reaching past this module for them is how a second stroke
// weight gets into the app.
export const Filter = light(_Filter);
export const Inbox = light(_Inbox);
// The change log's `flag` kind glyph (ui-spec §4.10) — the one kind whose tone
// stays `danger`, because a flag genuinely is a problem marker.
export const Flag = light(_Flag);
export const Gauge = light(_Gauge);
export const GitBranch = light(_GitBranch);
export const Github = light(_Github);
export const Globe = light(_Globe);
export const GripVertical = light(_GripVertical);
/** A Crew channel (`# methods`). The channel glyph and nothing else. */
export const Hash = light(_Hash);
export const HeartPulse = light(_HeartPulse);
export const History = light(_History);
export const Home = light(_Home);
export const Image = light(_Image);
/** A device key a person holds (Crew's Keys dialog). Deliberately not `Lock`,
 * which is the privacy tier and nothing else. */
export const KeyRound = light(_KeyRound);
// The three affiliation marks (`AffiliationBadge`); the tier mark is `Lock`,
// below. They live here for the reason every other glyph does —
// `light()` pins strokeWidth 1.5 and `currentColor` (§3.8b) — and they were the
// exception that proved why the wrapper exists: both badges imported these four
// straight from `lucide-react`, so they shipped at lucide's default
// strokeWidth 2 and read as a heavier, denser icon language than the 115 files
// drawing from this module. In the composer toolbar the affiliation glyph sat
// at 2px beside `Plus`, `Brain` and `Send` at 1.5px, on a *smaller* box, which
// nearly doubled its stroke-to-size ratio and made it look filled next to
// outlines.
//
// ⚠ **The glyph-to-meaning mapping is argued in `AffiliationBadge` and must not
// be re-decided here.** A laptop says where the work happens; a landmark is an
// institution; and `local` must never take a landmark, because that would file
// it as "an institution, but smaller" when it is in fact the most permissive
// affiliation there is (DR-26).
export const Landmark = light(_Landmark);
export const LandmarkIcon = Landmark;
export const Laptop = light(_Laptop);
export const LaptopIcon = Laptop;
export const Layers = light(_Layers);
export const Link = light(_Link);
/** A plan or to-do list (`todo__*` tool rows). */
export const ListTodo = light(_ListTodo);
export const Loader2 = light(_Loader2);
export const LoaderCircle = light(_LoaderCircle);
/**
 * **The one mark for the private privacy tier** (issue #56) — `PrivacyBadge`,
 * in both its pill and its dense form.
 *
 * ⚠ It is the same padlock the lock badge (`withPrivateBadge`, below) hangs on
 * every private chat glyph, and that is the whole point: a private chat, a private model
 * and a private extension are one fact — the tier that decides which models may
 * see the data — and were once drawn with three unrelated figures (a padlocked
 * bubble, a bare dot, and a shield). The shield is gone; do not reintroduce a
 * second privacy glyph for a fourth subject. Name the subject in words beside
 * the padlock instead (DR-53: one glyph, one meaning).
 *
 * Its older, unrelated uses — the Keychain notice, a permission prompt, an
 * `EACCES` artifact — are "this is locked to you", not the tier, and are why
 * the glyph reads correctly here in the first place.
 */
export const Lock = light(_Lock);
/** Disconnect / sign out of a connection. */
export const LogOut = light(_LogOut);
export const Maximize2 = light(_Maximize2);
export const MessageSquare = light(_MessageSquare);
export const MessageSquarePlus = light(_MessageSquarePlus);
export const MessageSquareText = light(_MessageSquareText);
/** Other chats: the `workspace__workspace_*` tool rows (read, steer, open). */
export const MessagesSquare = light(_MessagesSquare);
export const Monitor = light(_Monitor);
export const Moon = light(_Moon);
export const MoreHorizontal = light(_MoreHorizontal);
export const Music = light(_Music);
export const Package = light(_Package);
export const Palette = light(_Palette);
export const PanelLeftIcon = light(_PanelLeftIcon);
/** Toggles a right-hand details pane (Crew's channel details). The mirror of
 * `PanelLeftIcon`, which toggles the app sidebar. */
export const PanelRight = light(_PanelRight);
/** Attach a file to a message (Crew's composer Attach menu). */
export const Paperclip = light(_Paperclip);
export const Pause = light(_Pause);
export const PauseCircle = light(_PauseCircle);
export const Pencil = light(_Pencil);
export const Pill = light(_Pill);
export const Play = light(_Play);
// `Plus` means *add*: a provider, an extension, a variable, a schedule, a
// terminal. A new chat is `NewChat` (the compose mark) and a new window is
// `NewWindow`, so neither borrows the add mark (DR-53: one glyph, one meaning).
export const Plus = light(_Plus);
export const PlusIcon = Plus;
export const Puzzle = light(_Puzzle);
export const QrCode = light(_QrCode);
export const RefreshCw = light(_RefreshCw);
export const Rocket = light(_Rocket);
export const RotateCcw = light(_RotateCcw);
export const Save = light(_Save);
export const ScrollText = light(_ScrollText);
export const Search = light(_Search);
export const SearchIcon = Search;
export const Send = light(_Send);
/** A server path or a remote machine (Crew's server-path rows and hosting). */
export const Server = light(_Server);
export const Settings = light(_Settings);
export const Share2 = light(_Share2);
// `Shield` / `ShieldIcon` were the Private-tier mark and have been REMOVED
// rather than left exported unused. They had exactly one consumer,
// `PrivacyBadge`, which now draws the padlock every other private surface
// draws; an idle shield sitting in this barrel is an invitation to mark the
// next privacy surface with it and split the vocabulary again. Re-add it only
// for a meaning that is genuinely not "private tier" — and then say which.
export const Sliders = light(_Sliders);
export const SlidersHorizontal = light(_SlidersHorizontal);
export const Sparkles = light(_Sparkles);
// Opening a *new window* — the ⧉ mark the cohesion spec draws for the titlebar
// control (two offset windows). Deliberately not `Plus` (that is New chat),
// not `AppWindow` (that is the Applications route), and not `Copy`, whose
// geometry the spec's drawing actually matches but which already means copy.
/** "Open in a new window" — a second window appearing beside the first.
 *
 * Was `SquareStack`: two equally-sized offset squares, which is the same
 * figure `Copy` draws (36 call sites), so the titlebar control read as a
 * duplicate button. The alternatives collide too — an arrow leaving a box is
 * `ExternalLink` (20 sites, and it means "leave the app"), and a rounded
 * square with a stroke inside is `PanelLeftIcon`, this button's immediate
 * neighbour. PictureInPicture2 is the one glyph in the set that depicts the
 * actual outcome, and its two unequal shapes stay legible at 16px. */
export const NewWindow = light(_PictureInPicture2);
// The original 'Square' icon was visually a circle-with-square (stop button),
// which corresponds to lucide's StopCircle. Keep both names pointing to it.
export const Square = light(_StopCircle);
export const StopCircle = Square;
export const StopSquare = light(_Square);
/** A slash command (the mention popover's built-in commands, a workflow's
 * command). Not `Terminal` (a shell) and not `Zap`. */
export const SquareSlash = light(_SquareSlash);
/** **New chat**: the compose mark Codex and ChatGPT use. It frees `Plus` to
 * mean add. Crew keeps importing `Plus` for its own add actions, so the glyph
 * behind that name does not change. */
export const NewChat = light(_SquarePen);
export const Sun = light(_Sun);
export const Target = light(_Target);
export const Terminal = light(_Terminal);
/** Quote the selected text into the chat (the preview panel's selection action). */
export const TextQuote = light(_TextQuote);
export const Tornado = light(_Tornado);
export const Trash2 = light(_Trash2);
export const Upload = light(_Upload);
/** Invite or add a person. Deliberately not `Plus`, which means *new session*
 * (DR-53: one glyph, one meaning). */
export const UserPlus = light(_UserPlus);
export const Users = light(_Users);
export const Video = light(_Video);
// No direct 'Pipeline' in lucide; Workflow is the closest visual match.
export const Pipeline = light(_Workflow);
export const Wrench = light(_Wrench);
export const X = light(_X);
export const XIcon = X;
export const Zap = light(_Zap);

// ---------------------------------------------------------------------------
// Custom icons: hand-drawn glyphs that lucide-react does not have.
//
// Every one keeps the contract `light()` enforces for the Lucide set
// (design.md §3.9, DR-53): a 24 x 24 grid, `fill="none"`, stroke
// `currentColor` at 1.5, round caps and joins, all pinned AFTER the prop
// spread so no caller can add a second stroke weight or inject a hex. `size`
// is honoured the way Lucide honours it, and a glyph with no accessible name
// is `aria-hidden`, as Lucide's are. Each stamps `br-icon br-icon-<name>` as
// its identity class, which tests read the way they read `lucide-<name>`.
// ---------------------------------------------------------------------------

const A11Y_PROP = /^(aria-|role$|title$)/;

function isDecorative(props: object): boolean {
  return !Object.keys(props).some((key) => A11Y_PROP.test(key));
}

/** Drops the Lucide-only props an `<svg>` must not receive. */
function svgProps(props: LucideProps) {
  const {
    size = 24,
    absoluteStrokeWidth: _absolute,
    strokeWidth: _stroke,
    color: _color,
    ...rest
  } = props;
  return { size, rest };
}

const PINNED = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.5,
  strokeLinecap: 'round',
  strokeLinejoin: 'round',
  color: 'currentColor',
} as const;

/** A hand-drawn glyph with the `light()` contract. The body is static
 * geometry, so every instance of a glyph is the same figure. */
function glyph(name: string, body: React.ReactNode): React.FC<LucideProps> {
  const Glyph: React.FC<LucideProps> = (props) => {
    const { size, rest } = svgProps(props);
    const { className, children, ...attrs } = rest;
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width={size}
        height={size}
        viewBox="0 0 24 24"
        aria-hidden={isDecorative(attrs) ? true : undefined}
        {...attrs}
        className={['br-icon', `br-icon-${name}`, className].filter(Boolean).join(' ')}
        {...PINNED}
      >
        {body}
        {children}
      </svg>
    );
  };
  Glyph.displayName = `glyph(${name})`;
  return Glyph;
}

/** The speech bubble every plain chat draws. Rounder than Lucide's
 * MessageSquare (corner radius 3.5, not 2), with the tail folded into the left
 * edge, so the chat family reads as one silhouette. */
const BUBBLE =
  'M3 8a3.5 3.5 0 0 1 3.5-3.5h11A3.5 3.5 0 0 1 21 8v5.5a3.5 3.5 0 0 1-3.5 3.5H8.4a1.5 1.5 0 0 0-.98.37L4.25 20.1A.75.75 0 0 1 3 19.53Z';

/** A chat (`ENTITY_ICONS.chat`, the plain chat kind). */
export const ChatBubble = glyph('chat', <path d={BUBBLE} />);

/**
 * A Crew task chat: the bubble carrying Crew's channel mark (`Hash`, Crew's one
 * glyph for a channel). The row's title already reads `Crew · #methods · …`;
 * the glyph lets the eye find those rows without reading them.
 */
export const CrewChat = glyph(
  'crew-chat',
  <>
    <path d={BUBBLE} />
    <path d="M10 7.5v6.5M14 7.5v6.5M7.75 9.5h8.5M7.75 12h8.5" />
  </>
);

/** The same figure with the channel mark moved left, so the lock badge (bottom
 * right) does not cut through it. Only ever drawn under the badge. */
const CrewChatForBadge = glyph(
  'crew-chat',
  <>
    <path d={BUBBLE} />
    <path d="M8.25 7.25v6.5M11.5 7.25v6.5M6.25 9.25h7.25M6.25 11.75h5.75" />
  </>
);

/**
 * An agent: a sub-agent chat and a delegation row (`ENTITY_ICONS.agent`). A
 * calmer head than Lucide's Bot (no ear stubs), so it holds up at 14px and
 * under the lock badge. `Bot` stays exported unchanged for Crew.
 */
export const Agent = glyph(
  'agent',
  <>
    <rect x="4.5" y="8.5" width="15" height="11" rx="3.5" />
    <path d="M12 8.5V5.75" />
    <circle cx="12" cy="4.25" r="1.25" />
    <path d="M9.5 13v1.5M14.5 13v1.5" />
  </>
);

/**
 * Knowledge: a hub and four satellites on the diagonals. The earlier graph,
 * rebalanced (a smaller hub, larger satellites, shorter spokes) so it reads as
 * a graph at 14px instead of a smudge. It is the one Knowledge mark: the
 * composer's BookOpen and the "no primary base" Target go.
 */
export const KnowledgeGraph = glyph(
  'knowledge',
  <>
    <circle cx="12" cy="12" r="2.75" />
    <circle cx="5.5" cy="5.5" r="2" />
    <circle cx="18.5" cy="5.5" r="2" />
    <circle cx="5.5" cy="18.5" r="2" />
    <circle cx="18.5" cy="18.5" r="2" />
    <path d="M10.06 10.06 6.91 6.91M13.94 10.06l3.15-3.15M10.06 13.94l-3.15 3.15M13.94 13.94l3.15 3.15" />
  </>
);

/** The previous name of the Knowledge mark, kept so existing importers draw
 * the same figure as `ENTITY_ICONS.knowledge`. New code reads the entity map. */
export const KnowledgeIcon = KnowledgeGraph;

/**
 * A skill: a tile carrying a bolt, an ability the agent loads when it needs
 * it. Replaces `Layers`, which also meant "By model" in Usage and said nothing
 * about what a skill is. The mention popover's built-in rows, which used the
 * bare bolt, are slash commands and draw `SquareSlash`.
 */
export const Skill = glyph(
  'skill',
  <>
    <rect x="3.5" y="3.5" width="17" height="17" rx="4" />
    <path d="M12.75 7 9 12.5h6L11.25 17" />
  </>
);

/** Biorouter Copilot (desktop control): a display with a pointer on it.
 * `Monitor` stays the Settings App tab and the System theme. */
export const Copilot = glyph(
  'copilot',
  <>
    <rect x="2.5" y="3.5" width="19" height="13" rx="2.5" />
    <path d="M8.5 20.5h7M12 16.5v4" />
    <path d="M10 7.5l5.25 2.2-2.2.85-.85 2.2Z" />
  </>
);

// ---------------------------------------------------------------------------
// The lock badge: privacy as a SHAPE on any chat glyph.
// ---------------------------------------------------------------------------

/**
 * The padlock every private kind wears, bottom right, in the proportions of
 * `Lock` (the PrivacyBadge mark), so "private" is one figure on every surface.
 *
 * ⚠ Its ink is the family accent, authored in main.css as
 * `.br-icon-lock-badge { color: var(--text-accent) }`, while the body takes
 * whatever ink the row gives it (muted at rest). That keeps a spot of
 * Biorouter's colour on every private row without turning an all-private list
 * into a wall of coral glyphs. The group restates `stroke="currentColor"` so
 * the stroke resolves against the badge's own colour, not the body's.
 */
const LOCK_BADGE = (
  <g className="br-icon-lock-badge" stroke="currentColor">
    <path d="M16.5 15v-1.5a2 2 0 0 1 4 0V15" />
    <rect x="14.75" y="15" width="7.5" height="5.75" rx="1.25" />
  </g>
);

/**
 * Draws `Base` through a mask that clears the bottom-right corner (1.25 units
 * of air around the badge) and puts the padlock in the gap. Works for Lucide
 * wrappers and custom glyphs alike, because the base is nested as its own
 * 24 x 24 `<svg>`.
 *
 * ⚠ Build each private glyph ONCE at module scope, never inside a render: a
 * component created per render remounts its subtree every time.
 *
 * ⚠ The mask id comes from `useId`, sanitised: two glyphs on one page must
 * never share an id, or the second would be masked by the first one's mask.
 *
 * ⚠ The outer `<svg>` carries the pinned `stroke-width="1.5"`, which the tab
 * strip and artifact viewer suites read on every glyph.
 */
export function withPrivateBadge(
  Base: React.FC<LucideProps> | LucideIcon,
  name: string
): React.FC<LucideProps> {
  const Private: React.FC<LucideProps> = (props) => {
    const maskId = `br-private-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
    const { size, rest } = svgProps(props);
    const { className, children, ...attrs } = rest;
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width={size}
        height={size}
        viewBox="0 0 24 24"
        aria-hidden={isDecorative(attrs) ? true : undefined}
        {...attrs}
        className={['br-icon', `br-icon-${name}-private`, className].filter(Boolean).join(' ')}
        {...PINNED}
      >
        <defs>
          <mask id={maskId} maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">
            <rect width="24" height="24" fill="white" />
            <rect x="12.75" y="9.5" width="11.25" height="13.25" rx="2.75" fill="black" />
          </mask>
        </defs>
        <g mask={`url(#${maskId})`}>
          <Base x={0} y={0} width={24} height={24} aria-hidden />
        </g>
        {LOCK_BADGE}
        {children}
      </svg>
    );
  };
  Private.displayName = `withPrivateBadge(${name})`;
  return Private;
}

export const ChatBubblePrivate = withPrivateBadge(ChatBubble, 'chat');
export const CrewChatPrivate = withPrivateBadge(CrewChatForBadge, 'crew-chat');
export const AgentPrivate = withPrivateBadge(Agent, 'agent');

// ---------------------------------------------------------------------------
// LucideIcon type re-export — kept for consumers that import the type.
// ---------------------------------------------------------------------------
export type { LucideIcon, LucideProps };
