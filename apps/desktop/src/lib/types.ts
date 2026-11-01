// Shapes mirrored from the wipe-daemon REST API (see crates/wipe-core/src/model.rs).

export interface Comment {
  id: string;
  author: string;
  body: string;
  created: string;
  edited?: string;
}

/** A structured change event on a ticket (see wipe-core `Activity`). */
export interface Activity {
  ts: string;
  actor: string;
  /**
   * One of: created, moved, renamed, edited, priority, label-added,
   * label-removed, assigned, unassigned, attached, detached.
   */
  kind: string;
  detail?: string;
}

/** A single checklist / to-do item within a ticket (see wipe-core `ChecklistItem`). */
export interface ChecklistItem {
  id: string;
  text: string;
  done: boolean;
}

/** Which tickable surface on a ticket an operation targets: the worker-facing
 *  checklist (`ck-<n>`) or the reviewer-facing acceptance criteria (`ac-<n>`). */
export type ChecksKind = 'checklist' | 'acceptance';

export type AttachmentSource = 'media' | 'repo';

export interface Attachment {
  name: string;
  path: string;
  source: AttachmentSource;
  size: number;
  mime: string;
}

export interface Ticket {
  version: number;
  id: string;
  /** The decimal id from before the board's translation to hex ids. */
  legacy_id?: string;
  title: string;
  body?: string;
  priority?: string;
  labels: string[];
  assignees: string[];
  attachments: Attachment[];
  comments: Comment[];
  checklist: ChecklistItem[];
  acceptance: ChecklistItem[];
  activity: Activity[];
  created: string;
  updated: string;
}

export interface List {
  list: string;
  name: string;
  tickets: Ticket[];
}

export interface Board {
  board: string;
  /** Ticket id format: `hex` (`T-02A`) or a legacy decimal board (`T-42`). */
  ids?: 'hex' | 'decimal';
  commit?: string;
  lists: List[];
}

export interface Project {
  path: string;
  name: string;
}

export interface Health {
  ok: boolean;
  service: string;
  version: string;
}

/** User-global preferences surfaced by `GET/PATCH /api/config`. */
export interface AppConfig {
  accent?: string | null;
  theme?: string | null;
  default_identity?: string | null;
  prefer_default_identity?: boolean;
  /** Days a deleted ticket stays restorable in the trash before permanent purge. */
  trash_retention_days?: number;
}

/** A soft-deleted ticket sitting in the trash. */
export interface TrashItem {
  id: string;
  title: string;
  list: string;
  labels: string[];
  /** RFC-3339 deletion timestamp. */
  deleted_at: string;
}

export interface TrashListResponse {
  retention_days: number;
  count: number;
  trash: TrashItem[];
}

export interface CommitInfo {
  hash: string;
  short: string;
  author_name: string;
  author_email: string;
  date: string;
  subject: string;
}


export interface LabelDef {
  name: string;
  color?: string;
  description?: string;
}

export interface Definitions {
  version?: number;
  labels: LabelDef[];
  priorities: string[];
}

/** A forum post: the root of a thread or a reply at any depth. */
export interface ForumPost {
  id: string;
  author: string;
  body: string;
  labels: string[];
  refs: string[];
  attachments: Attachment[];
  created: string;
  edited?: string;
  replies: ForumPost[];
}

export interface ForumThread {
  version: number;
  id: string;
  title: string;
  root: ForumPost;
  created: string;
  updated: string;
}

export interface ForumThreadSummary {
  id: string;
  title: string;
  author: string;
  labels: string[];
  posts: number;
  created: string;
  /** Last-activity timestamp (most recent post in the thread) — drives the sort. */
  updated: string;
  /** Who made the most recent post (bumped the thread). */
  last_author: string;
  /** One-line preview from the root post. */
  snippet: string;
  /** Distinct authors who posted anywhere in the thread (for "who engaged" filters). */
  participants?: string[];
}

/** A flattened post returned by forum search. */
export interface ForumMatch {
  id: string;
  thread_id: string;
  thread_title: string;
  author: string;
  body: string;
  labels: string[];
  refs: string[];
  depth: number;
  replies: number;
  attachments: number;
  created: string;
  edited?: string;
}

export type IdentityKind = 'human' | 'agent';

export interface Identity {
  id: string;
  display_name: string;
  kind: IdentityKind;
}

export interface CreateTicketInput {
  title: string;
  priority?: string;
  list?: string;
  body?: string;
  labels?: string[];
  assignees?: string[];
}

/**
 * Partial ticket update. Omit a key to leave it unchanged. For `priority`,
 * pass `null` to CLEAR the value (JSON.stringify keeps null, drops undefined -
 * which is exactly the daemon's Option<Option<String>> semantics).
 */
export interface TicketPatch {
  title?: string;
  body?: string;
  priority?: string | null;
  labels?: string[];
  assignees?: string[];
}

// --- board sync (the wipe branch) ------------------------------------------

/** One typed board change (see wipe-core `sync::Change`). */
export interface SyncChange {
  /** `ticket`, `thread`, `list`, `label`, `board`, `file`. */
  scope: string;
  /** `T-003`, `F-2`, a list or label id, a file path. */
  id: string;
  /** created, deleted, moved, reordered, renamed, edited, commented, checked, posted. */
  kind: string;
  title: string;
  from?: string;
  to?: string;
  fields?: string[];
  count: number;
  detail?: string;
  by?: string[];
}

export interface SyncCommit {
  hash: string;
  short: string;
  author_name: string;
  author_email: string;
  date: string;
  subject: string;
}

/** Where the board stands against the wipe branch. */
export interface SyncStatus {
  /** `branch` (normal), `unmigrated` (board predates the branch), `tracked`
   *  (this checkout still tracks .wipe/), `none` (no git). */
  mode: 'branch' | 'unmigrated' | 'tracked' | 'none';
  branch: string;
  remote: string | null;
  board_dir: string;
  base: string | null;
  upstream: string | null;
  /** Uncommitted changes in the working copy. */
  local: SyncChange[];
  /** Changes on the branch not yet in the working copy. */
  incoming: SyncChange[];
  commits: SyncCommit[];
  unpublished: number;
  rewritten: boolean;
  fetched: string | null;
  fetch_error?: string;
}

export type SyncSide = 'ours' | 'theirs';

/** A disagreement the merge needs settled (see wipe-core `sync::Conflict`). */
export interface SyncConflict {
  id: string;
  entity: string;
  field: string;
  base: unknown;
  ours: unknown;
  theirs: unknown;
}

/** One commit of the wipe branch, placed relative to your board. */
export interface SyncLogEntry extends SyncCommit {
  /** `incoming`, `unpublished`, `base` (what your board is based on), `past`. */
  place: 'incoming' | 'unpublished' | 'base' | 'past';
}

/** What to take on its own (see wipe-core `sync::take::Entity`). */
export interface SyncEntity {
  scope: string;
  id: string;
  fields: string[];
}

export interface SyncResolutions {
  default?: SyncSide | null;
  picks: Record<string, SyncSide>;
}

export interface SyncPreview {
  board: Board;
  incoming: SyncChange[];
  conflicts: SyncConflict[];
  renumbered: { from: string; to: string }[];
}

export interface SyncResult {
  ok: boolean;
  conflicts?: SyncConflict[];
  applied?: SyncChange[];
  changes?: SyncChange[];
  merged?: SyncChange[];
  short?: string | null;
  message?: string;
  published?: number;
  renumbered?: { from: string; to: string }[];
  error?: string;
}
