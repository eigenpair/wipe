# Changelog

## 1.1.3 - unreleased

- **wipe supports git, and only git.** Identities and the people on a board
  come from git alone, and only inside a git repository; wipe no longer asks
  Plastic SCM, Mercurial, SVN, Fossil or Jujutsu who you are. Their user
  names didn't match your git identity, so a board moved from one of them to
  git ended up with the same person (or their agents) twice. Outside git a
  board is plain files, whatever hosts the folder: `wipe doctor`, the board's
  top bar and history say so in one line, and the Plastic-specific notes and
  `ignore.conf` check from 1.1.2 are gone - ignoring `.wipe/` in another
  version control is up to you.
- To keep a board's history and sync in a project that lives in another
  version control, give it a small git repo of its own in the same folder (a
  `.gitignore` of `/*` and `!/.gitignore`), run `wipe migrate`, and stop the
  other version control from tracking `.wipe/`.

## 1.1.2 - 2026-11-12

- **What's new, built in.** `wipe changelog` prints the release notes of the
  newest versions (`wipe changelog 1.0.0` for one, `--since 1.0.0` for
  everything after the version you came from, `--all`, `--json`). In the
  board, the version badge in the top bar opens the same notes, rendered and
  filterable. The changelog now goes all the way back to 0.1.0.
- **Boards in a Plastic workspace say so.** In Unity Version Control (Plastic
  SCM) - or Mercurial, SVN, Fossil, Jujutsu - there is no wipe branch: the
  board is plain files in `.wipe/` that travel with your checkins, as before
  1.0. `wipe doctor`, `wipe sync --check`, the board's top bar and the history
  drawer now tell you that, instead of a bare "not a git repository", and
  `wipe commit` / `sync` / `log` / `migrate` explain why they need git.
  `wipe doctor` also warns when Plastic's `ignore.conf` doesn't keep the
  board's cache (`.wipe/.cache`) out of checkins.

- **The tray's "Open board" works again.** A tray started at login (or hidden
  by a launcher) gives up its console window, and the console handles it kept
  were dead - so every program it started failed with "the handle is
  invalid": the browser for "Open board", and git for the board server it
  runs (your identity, sync). The tray now points its standard handles at
  `NUL` when it releases the console.
- **Double-click the tray icon** to open the board (a single click no longer
  does, so a double-click opens one tab, not two).
- **"Create shortcut"** in the tray menu, below "Start at login": puts a wipe
  shortcut with the wipe icon on your desktop. It opens the board, starting
  the tray first if it isn't running (`wipe tray --open`).

### Board sync hardening (an audit of commit, sync and history)

- **Taking something back from the history never silently discards your
  uncommitted work.** Bringing a ticket or post back as it was after an older
  commit replaced it outright - including edits you hadn't committed yet. Now
  it stops and names them: the board asks before replacing, `wipe restore`
  needs `--discard-local`. (Applying *incoming* changes one at a time already
  merged with your edits and still does.)
- **A rewritten wipe branch no longer causes false conflicts.** When the
  remote branch was force-pushed or recreated (even with the same content),
  edits made meanwhile clashed with values that hadn't changed. Your board's
  last synced state is now the merge base, so only real differences count.
- **Activity and comments merge in time order.** Entries both sides added
  were placed by which side merged, so a later rename could show above an
  earlier one; they now interleave by their timestamps.
- **A fresh clone stays clean.** An agent's first `wipe sync` (or `commit`,
  `restore`) registered it on the board and left "identities updated" as an
  uncommitted change; agents are now registered when they first write
  something.
- **Refresh fetches.** The board's Refresh button now asks the remote too, so
  what teammates just committed shows up at once instead of within a minute.
- Incoming renames show who made them.

## 1.1.1 - 2026-11-10

- **`wipe update` no longer hangs the shell that ran it.** When it restarts
  your tray, the tray is now launched fully detached - without the update's
  output handles. Before, a shell reading the update's output (a script, an
  agent's command runner, PowerShell redirecting to a file) waited until the
  tray exited, i.e. forever. Takes effect from the next update on: updating
  *to* 1.1.1 still runs 1.1.0's updater.

## 1.1.0 - 2026-11-09

> **Breaking for converted boards:** converting a board (**New ids** in the UI,
> `wipe board translate-ids`) rewrites its forum into a format wipe 1.0 can't
> read. Everyone who uses a board must update to 1.1 before it is converted.
> Both the UI and the CLI say so and ask for an explicit confirmation; agents
> are told to ask their user first. Boards that aren't converted keep working
> with 1.0 and 1.1 side by side.

### References: one `@` syntax for everything

- `@T-02A` (ticket), `@F-01C` (forum post) and `@claude` / `@ada@example.com`
  (people and agents) are recognised in every text - descriptions, comments,
  titles, checklist items, criteria, forum posts - and shown as chips. Hovering
  one shows a compact preview card; clicking follows it: inside a ticket the
  popup switches to the referenced ticket, on the board the card is scrolled
  into view and highlighted, a post opens its thread at that post.
- Typing `@` suggests as you type: people and agents first, then tickets and
  posts by what they say (titles and summaries first). `@T5` / `@F2a` search by
  id - the dash is put in for you. Arrows + Enter pick.
- **Mentions count like assignments**: the person mentioned gets a
  notification in the board UI, it reaches an agent's `wipe inbox` (reason
  `mentioned`) and its `[wipe]` updates, and a mention of you is highlighted.
- **Real page links**: `/T-023` opens a ticket, `/F-01A` a forum post,
  `/forum` the forum, so back/forward walk every jump and links can be shared.

### The forum, reworked

- Every post - opening post or reply - is its own entity with a hex id
  (`F-01A`) in its own file, headed by a **required one-sentence summary**
  (`--summary`, `-s`; `--title` still works).
- No built-in hierarchy: a post replies to the posts its text *opens* with
  tags for (`@F-01A @F-01C ...`, several allowed); any tag later in the text is
  a plain reference. Replying puts the tag in front of your text for you. The
  sidebar lists the posts that reply to nothing; deleting a post leaves its
  replies.
- Posts take attachments: paste, drop or pick files in the composer.
- Boards from before 1.1 keep their thread forum until converted - **New ids**
  in the UI / `wipe board translate-ids` now converts both ticket ids and the
  forum (posts numbered in the order written, old `F-3.1` ids kept as aliases,
  replies get their parent's tag and a summary from their first sentence, every
  `F-3.1` mention rewritten).

### Identities

- A person's identity is their git account: it can't be switched in the UI any
  more, only the alias you show up as (Board settings → You).
- Agents belong to the git account of the person running them, shown wherever
  identities are listed ("Ada's agent"). Picking an agent name another person's
  agent holds gives you your own variant (`claude02`); ids stay unique.
- When a sync meets two people's agents with one id, the incoming one keeps it
  and yours is renamed (`claude02`) with its tickets, posts and mentions -
  never a conflict.

### Editing and viewing

- Markdown controls everywhere: Ctrl/Cmd + B / I / U (+ E code, K link,
  Shift+X strike) in every text field, a compact formatting bar for
  descriptions, comments and posts, list continuation on Enter. Titles and
  checklist items render their inline formatting.
- The ticket popup shows its activity as a section of its own beside the
  ticket, scrolling on its own and wider, so long discussions never stretch the
  ticket and tables have room; the backdrop is darker and blurred.
- Image previews zoom with the mouse wheel around the cursor, a click zooms in
  on a spot, and a zoomed image pans by dragging (+ / - / 0 on the keyboard).
- The top bar shows the search shortcut (Ctrl/Cmd+F) beside the view switch.

## 1.0.1 - 2026-11-03

- **wipe moved to [eigenpair/wipe](https://github.com/eigenpair/wipe).** Repository links,
  the release downloads `wipe update` uses, the crate metadata and the "please
  report it" link now point there; the old `mflRevan/wipe` addresses redirect,
  so every earlier version keeps installing. The npm package is still
  `@mflrevan/wipe`.
- The release, npm-publish and website workflows run from the new home.

## 1.0.0 - 2026-11-01

### The wipe branch: one source of truth for the board

The board no longer travels inside your code branches. It lives on a dedicated
orphan branch, `wipe`, that holds nothing but board directories - and `.wipe/`
in each checkout is a working copy of it.

- **Explicit commits.** Board edits stay local until `wipe commit -m "..."`
  records them as one attributed commit on the wipe branch and publishes it.
  Nothing commits on its own (`board.autocommit` is gone), so the board's
  history reads as finished pieces of work. The default message summarizes the
  changes (`board: T-012 created, T-007 moved Todo → Done`).
- **`wipe sync`** applies your team's committed changes to your working copy
  and keeps your uncommitted ones on top; `wipe sync --check` shows both lists.
  `wipe commit` merges incoming changes first, and a push that loses a race
  fetches, merges and retries.
- **Merges per field, not per line.** Different fields of one ticket, new
  comments and forum posts, moves, reorders and checklist ticks all combine on
  their own; counters take the maximum and concurrent body edits merge by line.
  Ticket, thread, comment, checklist and reply ids that both sides allocated
  are renumbered on your side, references included. Only real disagreements -
  the same field set two ways, a delete against an edit, a card moved to two
  lists - stop the merge, listed with an id: settle them with `--take
  ours|theirs` or `--resolve <id>=ours|theirs`. Nothing is written until then.
- **Agents are told, every command.** After any wipe command an agent sees
  `[wipe] 3 board changes not committed (...) - wipe commit -m "..."` and
  `[wipe] 2 incoming board changes on the wipe branch (...) - wipe sync`.
  `wipe status --json` and `wipe doctor` carry the same counts. The check is
  cached, so it costs a command about a millisecond; a stale remote is fetched
  in the background.
- **The board UI shows it all.** The top bar offers **See N incoming
  changes**, which previews the board after applying them - new, moved,
  edited, commented and checked tickets each highlighted in their own color,
  with conflicts to pick a side for - and turns into close / **Apply**; and
  **Commit N changes**, which brings your changes forward with a message field
  and **Commit & publish**. Your own uncommitted changes are always marked
  subtly on their cards.
- **No checkout, no index.** wipe reads and writes the branch with git
  plumbing; your HEAD, index and working tree are never touched. Worktrees of
  a repo share one working copy, and a fresh clone checks the board out of the
  branch on its first `wipe` command.
- **One change at a time.** Every highlighted change in a preview has its own
  **Apply** (on the card, bottom right, on hover; and in the change list):
  take one incoming ticket, list or label and leave the rest incoming. From the
  CLI: `wipe sync T-004 list:review`. Your own new ticket under the same id
  makes room (renumbered), and once nothing is left incoming your board moves
  up to the branch by itself.
- **History, first class.** The history drawer is the wipe branch as a
  timeline: your uncommitted changes, incoming commits, *your board is here*,
  then the past. Opening a commit shows the board right after it with its
  changes highlighted; apply any of them (or all) to bring tickets, lists or
  labels back as they were - a revert you then commit like any change. From
  the CLI: `wipe log [--changes]` and `wipe restore T-003 --from <commit>`.
  This replaces the read-only rewind and the old all-branches commit graph.
- **Ids are settled by the branch.** Ids on the wipe branch never change; when
  both sides allocate the same id, the branch's ticket keeps it and your
  uncommitted one is renumbered - always, with no option to configure.
  Tickets and threads also carry a hidden random `uid`, so a merge tells "the
  same ticket" from "another ticket under the same number" by identity, not by
  content.
- **Your uncommitted changes look like drafts**: a dashed outline in the
  change's color and a dot by the id.

### Moving to 1.0

- Run **`wipe migrate`** once per repo: the branch is seeded from the board as
  last committed with your code (uncommitted edits stay uncommitted changes),
  published, and `.wipe/` is untracked and ignored on the current branch in a
  small commit. Other clones get the board from the branch on their next
  command. On an older branch that still tracks `.wipe/`, `wipe migrate`
  untracks it there and merges that branch's board edits in as local changes.
  The board UI offers the same move as a button.
- `wipe init` in a git repo puts the new board straight onto the branch.
- Boards outside git keep working as plain files.
- `board.autocommit` is removed; `wipe config set board.autocommit` says so.

### Development builds per directory

- **`wipe dev link [DIR] --bin <wipe>`** pins a directory to another wipe
  binary: every `wipe` run inside it - by you or by an agent, `-C` included -
  hands the whole command to that binary (optionally with its own config dir,
  `--config-dir`). `wipe dev unlink` / `wipe dev list` manage the pins, which
  live in your user config (`links.json`), never in a repo. Contributors get
  a ready sandbox with `node scripts/sandbox.mjs setup` (see CONTRIBUTING).
- A fresh clone nested inside another board's directory now gets its own board
  from its wipe branch instead of the enclosing one.
- **`WIPE_CEILING_DIRS`** (a path list, like git's `GIT_CEILING_DIRECTORIES`):
  board discovery never walks above these directories. A pinned directory sets
  it for its binary, so nothing in a sandbox reaches the board of the project
  it lives in.

## 0.4.6 - 2026-10-28

### `wipe update`

- **Updates wipe the way it was installed.** `wipe update` looks at where the
  running binary lives and updates through the matching channel: `npm install
  -g --prefix <that prefix>` for a global npm install (the right prefix even
  with several Node versions), `npm install` in the project for a project
  dependency, the release's shell / PowerShell installer when it left its
  receipt, or `cargo install wipe-cli` for a crates.io install. Builds from
  source get told how to rebuild instead.
- **Closes what would keep the old version running.** Other wipe processes
  started from the same installation - a `wipe tray`, a `wipe serve` - are
  listed and closed after a confirmation (`--yes` skips it; without a terminal
  it stops and says so). A closed tray is started again on the new version.
- On Windows the running binary moves itself aside so the update can replace
  it, and is put back if the update fails.
- `--check` shows the channel, the exact command, the latest version and what
  runs from the install, without changing anything; `--force` reinstalls the
  current version; `--json` keeps stdout to one result object.
- The daily "newer version available" notice now points at `wipe update`.

## 0.4.5 - 2026-10-28

### Fixes

- **No more console windows flashing on Windows while the board is open**
  ([#2](https://github.com/mflRevan/wipe/issues/2)). The tray and a detached
  `wipe serve` run without a console, so every `git` (or `cm`, `hg`, `jj`,
  `tailscale`, ...) they started got a console window of its own - several per
  page load or ticket save, since identities and the commit graph are read from
  the VCS. All external tools now start through one helper that sets
  `CREATE_NO_WINDOW`; output is still captured, and nothing changes in a
  terminal.

## 0.4.4 - 2026-10-21

### Board awareness for agents, without hooks

Agents don't look at the board on their own, so tickets filed for them and
comments on their work went unnoticed until someone told them. Now **every
wipe command reports what others changed since that identity's previous
command**, appended on stderr:

```
[wipe] 3 board updates since your last wipe command:
  - ada commented on your ticket T-001 "Set up CI": "please also cache the cargo registry @bot"
  - your ticket T-001 "Set up CI" moved Todo → In Progress by ada
  - 2 new tickets in Todo by ada: T-003 "Login crashes on empty password", T-004 "Flaky test in CI"
```

- **What is reported**: tickets anyone else filed (grouped per list); on
  tickets assigned to you or subscribed to: moves, comments, edits, deletion,
  and being assigned or unassigned; @mentions in comments and forum posts;
  replies in forum threads you started, replied in or subscribe to. Never your
  own actions.
- **Each change once**, whatever command runs next and in whatever shell: the
  board snapshot is kept per identity (gitignored, under `.wipe/.cache`), so it
  works in harnesses that start a fresh shell per command, and changes that
  arrive by `git pull` are reported too. The first command on a board only
  records the baseline. At most 8 lines, then a pointer to `wipe inbox`.
- **What's next.** Closing, submitting or approving a ticket (or moving it to
  the done / review list) ends with the open tickets still assigned to you:
  `[wipe] still assigned to you: T-003 "Crash: add() fails on strings" (Todo)` -
  the moment an agent decides what to do next.
- **`wipe status`** shows the same under "Since your last wipe command" (and as
  `updates` in `--json`); `inbox` and `watch` mark what they showed as seen.
- Costs about 2 ms per command on a 600-ticket board when nothing changed.
  `--json` stdout is unchanged (the lines go to stderr). `WIPE_NO_UPDATES=1`
  turns it off.
- `wipe ticket list` takes `-l` for `--list`, like `ticket create`.

### Fixes

- **Video and audio attachments can seek again.** The board server sent
  attachments as one plain response and ignored HTTP `Range` requests, so
  browsers could play and pause a video but not skip, rewind or drag the
  timeline. Attachments and local-file previews are now served with byte-range
  support (206 partial content, `Accept-Ranges`) and streamed from disk instead
  of read into memory whole.
- **Windows login autostart now really starts the tray.** Upgrades never
  replaced a login entry written by an older wipe - the pre-0.3.7
  `wipe-autoserve.cmd` (a console window running `wipe serve`) or a pre-0.4.1
  `.vbs` (hidden `wipe serve`, no tray) - because wipe already counted it as
  "autostart on" and left it alone. Any wipe command now rewrites a stale entry
  once into the hidden tray launcher, keeping the install it pointed at (or
  switching to the running one if that exe is gone).
- **`wipe tray` drops a console window it was given.** Started from Explorer, a
  shortcut or the Run key, the tray releases the console window Windows opened
  for it instead of leaving it up next to the tray icon.
- **`wipe` from PowerShell skips Node.** PowerShell picks npm's `wipe.ps1` shim
  over the native `wipe.exe` that global npm installs place beside it, so every
  call from PowerShell (including agent harnesses that use it as their shell,
  like opencode) still started Node first. The installer now adds a short
  preamble to `wipe.ps1` that hands off to `wipe.exe`; `npm uninstall` still
  removes the shim as before. Reinstall with `npm i -g @mflrevan/wipe` to get it.

## 0.4.3 - 2026-10-21

The agent release: every read view was rebuilt to cost an agent as little
context as possible and to say how to go deeper, search and change
notifications became first-class commands that work in any agent harness, and
boards whose id translation was finished by hand are now repaired.

### Agents: less context, discoverable depth

Measured on three real boards (60-128 tickets) with a scripted agent session;
see [docs/OUTPUT-AUDIT.md](docs/OUTPUT-AUDIT.md) for the full before/after table.

- **`ticket show` is bounded.** Facts on one line, the body, checklists,
  attachments by name and size, the newest 3 comments and the latest commits -
  long bodies and comments are clipped with a count of what was left out, and
  the view ends with the flags that reveal the rest (`--comments all` or `N`,
  `--activity`, `--full`). The activity log is now opt-in. A 44-comment ticket
  went from 43 KB to 2.5 KB (-94%); JSON follows the same selection and drops
  internals (`version`, `next_*` counters, nanosecond timestamps).
- **Leaner rows.** `status` / `ticket list` JSON rows default to id, title,
  list, labels, assignees, priority and blockers (`--fields` adds progress,
  timestamps, bodies): -25 to -41%. `status` caps a list at 60 rows and
  `ticket list` at 100, each with a pointer to the rest; a single-list
  `ticket list` no longer repeats the list on every row.
- **Inbox, forum list, search** have default limits (20, 25, 10) with "N more"
  hints; the inbox groups events by ticket (title once) and collapses bursts
  (ten label edits by one person are one line). `inbox --all` no longer
  requires an identity.
- **Help is shorter.** Subcommand help no longer repeats the global plumbing
  flags (`-C`, `--agentid`, `--echo`, `--pretty`; listed once in `wipe --help`),
  and the root help lists everyday commands first (-30% per help page).
- **The skill file is a bootstrap, not a manual** (9.2 KB -> 2.5 KB): the CLI's
  own hints carry the rest.
- `wipe ticket move T-02A review` - the target list can be positional
  (`--to` still works).

### Search

- **`wipe search <query>`** searches tickets and forum posts together, ranked
  (title hits first), and prints the one snippet that shows *why* something
  matched (`body: …`, `comment c-3 by ada: …`, `checklist ck-2: …`). Query
  syntax: words (all must match), `"phrases"`, `-exclude`, ticket ids in any
  spelling, `label:`, `list:`, `@who` / `assignee:`, `author:`, `priority:`,
  `is:open|done|blocked|ready|assigned|unassigned`. `--tickets` / `--forum`
  narrow it; `--json` returns the same hits.
- `wipe ticket list --search <query>` combines the same search with the list
  filters.

### Notifications for agents: `wipe watch`

- **`wipe watch [targets]`** waits for changes and prints each as one line:
  `T-02A moved review → todo · ada · Fix the foam`, `T-02A commented · ada ·
  Fix the foam: "please re-check the edges"`. Targets are tickets, lists,
  forum threads or `forum` (none = the whole board); `--on` picks events
  (created, moved, commented, edited, labeled, assigned, blocked, attached,
  checked, deleted, posted), `--from` / `--to` match moves out of / into a list
  ("tell me when it leaves review"), `--by` filters by actor.
- **Works in every harness through plain shell semantics**: streamed, each
  stdout line is a notification (monitor tools); with `--until` it exits 0 on
  the first match, so a background task's completion is the notification; with
  `--timeout 30m` it is a safe blocking call (exit 124 when nothing happened).
- **Never your own changes**, nothing for the current state, and changes that
  arrive by `git pull` are reported too. `--json` streams NDJSON.
- Checklist and acceptance-criteria ticks are now recorded in the activity log
  (`checked` / `unchecked`), so they show up in watch, the inbox and the UI.

### Guardrails

- **Board text is sanitized before it is printed**: terminal escape sequences,
  control characters and bidi overrides are removed; pasted `data:…;base64,…`
  images and long encoded blobs become `[image/png base64, 34 KB omitted]`;
  unbroken runs over 400 characters are cut. Applies to titles, bodies,
  comments, checklists, forum posts, search snippets and events, in text and
  JSON.
- Forum post refs are stored in canonical form (`--ref t2a` -> `T-02A`, an old
  `T-42` -> its hex id).

### Ticket ids: repair and hardening

- **Boards whose translation was finished by hand are repaired.** When 0.4.1
  left a board half-converted and `board.json` was switched to hex manually,
  the trash and the forum kept decimal ids (`T-1.json` in the trash, `refs:
  ["T-30"]` and "see T-40" in threads). On the first command after upgrading,
  wipe converts those leftovers once (trash entries and stray ticket files
  renamed, forum refs and mentions, subscriptions and relations rewritten);
  ids that could be either format are left alone and reported.
  `wipe board translate-ids` on a hex board runs the same repair on demand.
- Fixed the "translation interrupted" error messages, which contained a run of
  spaces / a raw line break from a broken line continuation.

### Board UI

- **Large lists stay smooth** (measured on an 850-ticket board with a
  451-card list): dragging within the list went from 19 long tasks (1.7 s
  blocked) to none, and the worst frame from 272 to 88 ms; drags across lists
  and onto the trash have no long tasks; search keystrokes are about twice as
  fast. Card reorder animations are one batched pass over the visible cards
  instead of a layout per card, off-screen cards in long lists skip layout and
  paint, search only highlights matching cards and builds its index only while
  open, and live updates no longer re-render every card when labels or
  identities didn't change.
- The activity log names checklist ticks and blockers instead of showing the
  raw event kind.

### Hardening

- A ticket file that cannot be read - typically git merge-conflict markers
  left in it - no longer silently disappears from `status` and lists: it is
  skipped with a warning, and every error names the file and says what to do
  ("has git merge-conflict markers - resolve the conflict in that file").
- `wipe status | head -1` (a reader closing the pipe early) exits quietly
  instead of panicking; any other internal error is one line with where to
  report it, not a backtrace.
- `ticket create --list <unknown>` names the board's lists, like `move` does.
- A `wipe watch` takes its snapshot before announcing itself, so a change
  right after the "watching ..." line is never missed.
- Short pre-translation ids (`T-12`) resolve with one file read instead of
  loading every ticket.
- UI quick-jump: typing a hex id that doesn't exist (`T2AF`) no longer jumps to
  the legacy ticket `T-2`.

### Changed (JSON consumers)

- `ticket show --json`, `comment list --json`: comments are
  `{"id","by","at","body"}` (was `author` / `created`); the activity log is only
  included with `--activity` / `--full`.
- `inbox --json`: `{"total","items":[{"id","title","why"?,"events":[{"at",
  "event","by","detail"?}]}]}`; event names are `commented`, `moved`, `labeled`,
  `assigned`, `checked`, `posted`, ... (was `comment` / `activity` / `forum`).
- Default JSON rows of `status` / `ticket list` drop `comments`, `checklist`,
  `criteria`, `updated` and `last_by` (ask for them with `--fields`).

### Tests

- The suite was pruned from 129 to 68 tests and reorganized by area: duplicated
  and trivia tests were merged or removed, every regression guard was kept, and
  the new agent surface is covered end to end (search, bounded `show`, the
  guardrail matrix across eight views in text and JSON, `watch --until` /
  timeouts / self-filtering, the hex-board repair). Stress tests prove the same
  properties with fewer processes; `cargo test --workspace` runs in about 5 s.

## 0.4.2 - 2026-10-01

Hotfix for the ticket-id translation added in 0.4.1. If you translated a board
with 0.4.1, run any `wipe` command in it (or open it in the board UI) once
after upgrading - a half-converted board is repaired automatically.

### Fixed

- **`wipe board translate-ids` (and the New ids button) could leave a board
  half-converted.** On Windows, replacing `board.json` fails while another
  process (an editor, the search indexer, a virus scanner, a sync client) has
  it open; 0.4.1 had already renamed every ticket file by then, so the board
  kept its old decimal cards pointing at files that no longer existed, and
  re-running failed with "unexpected ticket id `T-01A` on a decimal board".
  The translation is now transactional: the complete result is computed first
  and saved as one journal, then applied with `board.json` written last.
  Applying is idempotent, so an interrupted translation finishes on the next
  `wipe` command, the next board load in the UI, or a re-run - never redone
  (which would re-map ids that are already translated).
- **Boards left half-converted by 0.4.1 are repaired** the same way: tickets
  already carrying a `legacy_id` are kept as they are, tickets 0.4.1 deleted but
  never wrote back are restored from its staging snapshot
  (`.wipe/.cache/translate-*`), and the cards, forum, subscriptions and trash
  are translated to match.
- Translating a board with 256 or more tickets could lose a trash entry: old
  `T-100` and the new hex `T-100` (ticket 256) shared a file name.
- Writes on Windows retry for a moment when the target file is briefly held
  open by another process, instead of failing immediately.

## 0.4.1 - 2026-10-01

### Ticket ids

- **New boards use fixed-width hex ticket ids:** `T-001` ... `T-0FF`, `T-2AF`
  (three or more upper-case hex digits; `board.json` records `"ids": "hex"`).
  Short, uniform, and quick to type.
- **Any spelling resolves everywhere** (CLI, API, quick-jump): `T-02A`, `t2a`,
  `T-2A`, `T 2a`. CLI output always shows the canonical id.
- **Legacy boards keep working** with decimal ids (`T-23`). `wipe board
  translate-ids` (or the **New ids** button in the board UI, shown only on such
  boards) converts one: ticket files are renamed, and cards, relations,
  subscriptions, trash entries, forum refs and `T-<n>` mentions in titles,
  bodies, comments, checklists and activity are rewritten. Each ticket keeps its
  old id as `legacy_id`, so `T-23` (in old notes or commit messages) still
  resolves, and `ticket show` also lists commits that mention the old id. The
  first CLI run on a legacy board offers the translation once: an interactive
  y/N question, or a one-line stderr notice for agents and scripts (never a
  blocking prompt). The staged translation is crash-safe.

### Board UI

- **Ticket quick-jump:** with nothing focused, type `T` and the ticket's digits
  (`T2A`; the dash is optional). The query shows large in the middle of the
  screen, the matching card glows and scrolls into view, Enter opens it, Escape
  or any click dismisses. Ticket ids are now shown on every card.
- **Board search (Ctrl/Cmd+F)** replaces the browser's find-in-page: it searches a
  cached index of every ticket's id (and pre-translation id), title, description,
  labels, priority, assignees and authors (ids and display names), comments,
  checklist and acceptance items, and attachment names; all terms must match,
  "quoted phrases" stay together, accents are ignored. Non-matching cards hide
  live, per-list counts show `3/108`, hits in titles and labels are highlighted,
  and cards that matched elsewhere say so. The search survives opening tickets;
  Escape or the close button (marked `esc`) ends it. In the forum view, Ctrl/Cmd+F
  focuses the forum's own search.
- **Notifications:** watch the whole board, a list (list menu), a ticket (bell in
  the ticket header), a forum thread, or the whole forum (the bell menu in the
  top bar). Changes by others - new cards, moves (into or out of a watched
  list), comments, assignments, labels, edits, completed checklists or
  criteria, forum replies - arrive as native system notifications while the
  tab is in the background (click to open the ticket or thread) and as in-app
  toasts while you're looking; bursts collapse into one summary. Your own edits
  never notify you. Watches are kept per board in the browser. System
  notifications need https or localhost; on a LAN address (e.g. a phone) the
  in-app alerts are used.
- **Faster, smoother board on large boards** (measured on a 127-ticket board
  with 1.1 MB of tickets, dragging a card across a 108-card list):
  - the board poll uses ETags: the daemon caches the payload by file
    fingerprint and answers unchanged polls with `304` (0 bytes, ~1 ms) instead
    of re-reading every ticket and sending ~730 KB every half second, and the
    UI no longer `JSON.stringify`s the whole board twice per poll to detect
    changes;
  - the daemon also listens on `::1` / `::`, so `http://localhost` no longer
    waits ~200 ms per request for a failed IPv6 attempt before falling back to
    IPv4 (most noticeable on Windows);
  - unchanged tickets keep their identity across refreshes, the board keeps its
    lists as raw (non-proxied) state, and reorder animations run only for cards
    in view.
  Result: the drag that froze 0.4.0 for 389 ms now has no long tasks (worst
  frame 49 ms), and search filtering settles within a frame or two.

### Tray app

- **`wipe tray`** (Windows, macOS) runs the board server as a real desktop app:
  an icon in the notification area / menu bar whose menu opens the board, opens
  a phone page with QR codes for every network address, copies the phone link,
  toggles start-at-login, and quits cleanly. Left-click opens the board. Only
  one runs per user (a second `wipe tray` just opens the board). Started from a
  terminal it moves to the background. Login autostart now launches the tray on
  Windows and macOS instead of a hidden `wipe serve`; Linux keeps the systemd
  user unit.
- The daemon serves `/connect` (this machine only): the board's network URLs
  with the access token as links and QR codes; `wipe serve` points at it.

### Fixed

- `wipe serve` / `wipe scan` find boards on every local drive, not just under
  your home folder: the default scan roots are now home plus each fixed drive
  (`D:\`, `E:\`, ...; mounted volumes under `/Volumes`, `/mnt`, `/media` on
  macOS/Linux). Network and removable drives are left out; OS trees at drive
  roots (`Windows`, `Program Files`, `$Recycle.Bin`, ...) and `AppData` are
  skipped, and a root inside another (home inside `C:\`) is walked once.
  `wipe serve` now also scans when started inside a board (in the background,
  so startup stays instant), and the UI's project switcher refreshes its list
  each time it opens. Configured `scan.roots` still replace the defaults.
- `wipe trash restore|purge` built the trash file path straight from the typed
  id, so `wipe trash purge ../../board` could delete `.wipe/board.json`; trash
  ids are now matched against the trash entries themselves (any id spelling).
- UI: dropping a card on the trash bin no longer flies it back to its list for a
  moment before it disappears. The drop-return animation is skipped while the
  pointer is over the bin, and the card leaves the board immediately (the delete
  itself was already correct).

## 0.4.0 - 2026-09-29

Driven by twelve days of field feedback from an agent (Claude) and a human
sharing one board on a real project, plus human-side QoL work. The theme: less
output, safer text input, forced identity, a first-class review loop, and a board
you can run from your phone.

### Breaking changes

- **No default identity.** CLI writes are refused until an identity is chosen for
  the session (`wipe identity use <id>`, `$WIPE_AGENT`, `--agentid`, a per-command
  `--author`, or `$WIPE_AUTHOR`). The repo's VCS user, the board's
  `default_author`, and `identity.default` are no longer used by the CLI (the
  board UI still uses them). The refusal lists the board's identities, how to pick
  one, and tells agents to ask their user rather than guess. Reads never need an
  identity. `$WIPE_STRICT_IDENTITY` is now redundant (always strict).
- **Sessions are per terminal tab / agent session.** The shared `default` session
  key is gone (it made one `identity use` apply to every terminal and every agent
  on the machine). Keys come from `$WIPE_SESSION`, the agent harness's session id
  (e.g. Claude Code), the terminal's tab id (Windows Terminal, iTerm, tmux, ...),
  or the launching shell process. An agent started from a human's terminal never
  inherits the human's identity. Old bindings are ignored.
- **Compact JSON.** `--json` prints one compact line (about half the bytes);
  `--pretty` restores indentation. Object keys now keep their natural order.
- **`wipe status --json` is compact:** per list `{id, name, count, tickets:[row]}`
  with short rows; the done list collapses to `{count, collapsed:true}`. Use
  `--all` to expand done and `--full` for the old whole-content dump (whose list
  key is still `list`).
- **`wipe ticket list --json` returns compact rows** (`--full` for whole tickets).
- **Write commands return short receipts** (`{"ok":true,"id":"T-3","list":"todo"}`,
  37-80 bytes) instead of the whole ticket; `--echo` returns the full object.
- **`wipe serve` listens on all local networks by default** (see below). Remote
  clients need the access token; this machine does not. `daemon.expose` values are
  now `lan | local | tailscale | proxy`; the old `none` is read as `lan` and
  rejected by `config set` (use `local`).
- `wipe inbox` and `wipe forum search` return at most 50 items by default
  (`--limit 0` for all; inbox JSON adds `total`).
- MSRV is now Rust 1.89.

### Agent experience

- **Long text from files or stdin** everywhere text goes in: `--body-file <path>` /
  `--body -` on `ticket create|edit`, `comment add|edit`, `forum post|reply|edit`;
  `--comment-file` on `ticket edit`; `--message-file` on `ticket submit|reject`.
  File/stdin text loses a UTF-8 BOM and trailing newlines. Fixes silently
  truncated multi-line bodies on Windows.
- **Windows npm shim:** global npm installs now also place the native `wipe.exe`
  next to npm's shims, so `wipe` resolves to the binary instead of `wipe.cmd`
  (cmd.exe cuts arguments at the first line break). Launches through the node
  shim are marked, and `wipe doctor` explains the risk and the fix.
- **`wipe ticket edit` does everything in one call:** title, body, priority,
  `--label`/`--remove-label`, `--assignee`/`--unassign`, `--blocked-by`/`--unblock`,
  `--to <list>`, and `-m` comment - validated before anything is written.
- **Positional forms:** `wipe ticket create "Title" --list todo`,
  `wipe comment add T-1 "text"`; `wipe label assign|remove T-1 app bug` takes
  several labels; `-t`/`-b` short flags accepted on `ticket edit`.
- **`wipe ticket list` filters:** repeatable `--list` / `--exclude-list`,
  repeatable `--label` (all must match), `--assignee <who|me>`,
  `--since <RFC-3339 | 2026-09-01 | 30m | 12h | 2d | 1w>`, `--ready`, `--blocked`,
  `--fields id,title,...`, `--limit`. Unknown lists/fields fail with the valid set.
- **`wipe ticket show`:** lists commits whose messages mention the ticket id
  (`--no-commits`); `--comments N`, `--comments-only`, `--no-activity` trim it.
- **Original notes are kept:** the first time someone other than the creator
  rewrites a ticket's title or body, the creator's wording is frozen in `original`
  (shown by `ticket show`), so turning a human's raw note into a ticket is safe.
- **Status footer / hints:** human `status` ends with unread count and a pointer
  to `ticket list`; JSON status carries `identity` and `unread`.
- **Commit story:** until `wipe commit` has been used once on a machine, every
  board write prints a one-line stderr hint to record it (stdout JSON untouched);
  `wipe doctor` reports uncommitted board files. (`board.autocommit` remains.)
- **`board.json` churns less:** card moves, creates, deletes and trash no longer
  rewrite `board.updated`, so parallel worktrees conflict less.
- **`wipe inbox --all`:** everything anyone else changed on the board (reason
  `board`), for "what happened while I was away"; `--since` accepts the same
  forms as `ticket list`.
- **Forum digest:** `wipe forum pin|unpin F-n` and `wipe forum digest
  [--max-bytes 4000]` - a size-bounded Markdown summary of pinned threads, made to
  be loaded into an agent's context at session start.
- **Skill guide rewritten** around a start-of-session recipe (`whoami` → `inbox
  --unread` → `ticket list --exclude-list done`), the identity rules, file/stdin
  text, receipts, the review loop, dependencies, and committing; it now states
  that `.wipe/` files are internal.

### Human + agent workflow

- **Review loop:** `wipe ticket submit T-1 -m "..." --tested ... --untested ...`
  (comment with Tested / Not tested sections + move to the review list + the
  submitter is subscribed), `wipe ticket approve T-1 [-m] [--force]` (refuses while
  acceptance criteria are unticked), `wipe ticket reject T-1 -m "reason"` (reason
  required; back to the rework list). Workflow lists are detected (a "review" list,
  `done`, `todo`) or set with `board.review_list`, `board.done_list`,
  `board.rework_list`.
- **Dependencies:** `wipe ticket block|unblock T-5 --by T-3`, `--blocked-by` on
  create/edit; cycles are refused; blockers stop counting once done;
  `ticket list --ready` / `--blocked`; rows and `status` show open blockers.

### `wipe serve` and the phone

- Default exposure is the local network (`0.0.0.0`) with token auth for remote
  clients; requests from this machine skip the token (except in `proxy` mode).
  New flags: `--expose <mode>`, `--local`, `--tailscale` (binds loopback + this
  machine's Tailscale address, prints the MagicDNS URL when available),
  `--host <ip>`, `--no-qr`.
- Prints one URL per network interface and a terminal **QR code** for the phone.
- The access token is kept per machine in the user config dir (`serve-token`),
  never in the git-tracked board (a token already in `settings.json` is honored).
- Cross-origin API access in exposed mode is limited to this machine's own
  front-ends (desktop app, local dev server).
- **Mobile UI:** two-row header, one list per screen with swipe snapping, touch
  drag only after a short press (so swipes scroll), card menus visible without
  hover, full-screen ticket and create dialogs, single-pane forum with a back
  button, safe-area aware.

### UI

- The ticket title field grows with its content while editing (all wrapped lines
  visible) in both the ticket modal and the create dialog; the description editor
  grows too.
- Pasted or picked media shows an **in-place preview** before the card exists
  (images, video, audio, text/PDF tiles) and right under the field it was pasted
  into on existing cards; tap any preview or attachment to inspect it full-screen
  (zoom, download). Pasted local paths preview via the daemon (this machine only).
  Files can be dropped onto the create dialog.

### Reliability and security

- **Board write lock:** every CLI write and every UI mutation holds a per-board
  lock, so concurrent agents / the UI can no longer lose tickets, lose comments,
  or allocate the same ticket id (all three reproduced by the new stress tests
  without the lock).
- Attaching a file *by path* (and the new path preview) is refused for remote
  clients, so a token holder on the network can't make the daemon read arbitrary
  host files.
- Remote clients can't claim an identity in exposed mode; local ones can.

### Tests and docs

- New integration suites: `agent_ux` (14 end-to-end scenarios for the features
  above) and `stress` (concurrent creators/commenters/mixed writers, output-size
  bounds on a 300-ticket board); shared test harness that scrubs ambient identity
  and session env.
- New unit tests for commit mention matching, blockers/cycles, original notes,
  workflow list detection, `board.updated` stability, `--since` parsing, field
  selection, serve bind plans / URLs / QR, loopback trust, and CORS origins.
- `docs/OUTPUT-AUDIT.md`: measured bytes / tokens / time for every routine command
  at 10 / 100 / 500 tickets, plus every truncation and compaction rule; regenerate
  with `cargo test -p wipe-cli --release --test stress -- --ignored output_audit`.
- README, website CLI reference, and harness scripts updated for 0.4.

## 0.3.14 - 2026-07-15

### Fixes

- **Dragging cards is smooth again.** In 0.3.13 the "shrink over the trash"
  effect fought the drag library, so a dragged card snapped between lists and
  lagged behind the pointer. The shrink now applies to the card inside the
  dragged copy, so cards follow the cursor cleanly and still shrink over the
  trash.

## 0.3.13 - 2026-07-15

### Trash

- **Dragging to the trash always works.** Instead of relying on a small drop
  zone, the board now follows the pointer: while you drag, the card shrinks to
  0.7x whenever the pointer is over the bin (releasing there deletes it) and
  grows back when you move away. Deleted tickets still go to the restorable
  trash.

### Forum sidebar

- **Resizable.** Drag the divider to set the sidebar's width; it is remembered
  across sessions and the contents reflow to fit.
- **Denser thread rows.** One-line title and one-line snippet, cut off with an
  ellipsis to fit the sidebar.
- **Grouped by activity.** Threads are split under quiet dividers: Today, This
  week, This month, then by month.
- **Clearer indicators.** The post count is a distinct badge with an icon;
  author and time are right-aligned.
- **Filters.** The search bar gains a filter panel: filter by who took part in
  a thread (with an "only as original poster" toggle) and by labels. The text
  search now narrows the thread list, and the toggle shows how many filters are
  active.

## 0.3.12 - 2026-07-15

### Tickets

- **Deleting a ticket moves it to the trash.** Deleted tickets go to a
  per-board trash (not tracked by git) and can be restored for
  `trash.retention_days` (default 7, a global setting) before they are purged
  for good. New commands: `wipe trash list|restore|purge`,
  `wipe ticket delete --purge` to delete permanently, and
  `wipe ticket duplicate`.

### Board UI

- **A trash panel.** The trash can is always visible as a small icon, grows
  into a drop target while you drag, and opens on click: trashed tickets
  newest-first with Restore, delete-forever per item, empty-all, and the
  retention setting in place.
- **Card quick menu.** A menu in each card's top-right corner: Move to,
  Priority, Labels, Assign, Duplicate and Delete.
- **New cards open the full editor.** Labels, members, priority, description,
  checklist, acceptance criteria and attachments can all be set before the
  card exists; nothing is saved until you press Create.
- **Paste to attach.** Ctrl+V in a title or description attaches pasted media
  or a pasted local file path instead of inserting text. Pasting the same thing
  twice doesn't attach it twice.
- **Shorter project paths.** The project switcher shows compact paths (drive,
  ..., last folders) on one row beside the board name.

## 0.3.11 - 2026-07-14

### Fixes

- **List drag-to-reorder removed again.** The 0.3.10 list drag was broken: the
  column vanished on drop and came back unchanged after a reload. Reorder lists
  with Move left / Move right in the list menu, which saves reliably. Dragging
  tickets within and between lists is unchanged.

## 0.3.10 - 2026-07-14

### CLI

- **`wipe ticket create` requires `--list`.** Without it the command fails and
  shows the available lists and an example, so tickets no longer land silently
  in the leftmost list.
- **Fix the author of existing work.** `wipe comment edit`,
  `wipe comment reattribute`, `wipe ticket edit --author` and
  `wipe forum edit --author` correct work posted under the wrong identity
  without deleting and reposting it. Every correction is recorded.
- **`wipe commit [ref]`** commits `.wipe/` (or one ticket's file) as its own
  commit, attributed to wipe, without sweeping in anything else you have
  staged. Turn on `board.autocommit` to commit after every board change.
- **Stricter identity.** With `$WIPE_STRICT_IDENTITY` set, wipe refuses to
  attribute a write to the repo's VCS user in shared worktrees, and
  `wipe identity whoami` warns when identity sources disagree.
- **Subscriptions and an inbox.** `wipe subscribe`, `wipe unsubscribe` and
  `wipe subscriptions`, plus `wipe inbox [--unread|--since]`, show what others
  did on tickets you are assigned to, created or subscribed to. Assignees are
  subscribed automatically.

### Board

- **Exposed boards need a token.** When `wipe serve` is exposed over Tailscale
  or a proxy, the API and live updates require an access token (sent as a
  header or `?token=`). The token is part of the served URL and the board
  remembers it.
- Lists can be dragged to reorder them (reverted in 0.3.11).

## 0.3.9 - 2026-07-14

- **Delete comments.** `wipe comment remove`, or hover a comment in the board
  and delete it.
- **Drag a ticket to the trash.** A delete zone appears while dragging and
  glows when a release would delete. Lists that still hold tickets can now be
  deleted too, after a confirmation.
- **A better forum view.** Threads are sorted by latest activity, and each row
  shows a snippet, labels, the latest author, a relative time and the post
  count. Open threads get a header with their details.
- **`$WIPE_AGENT` for agents running side by side.** Set per terminal, it
  takes priority over the shared session file, so several agents on one
  machine each get credit for their own work.
- The board shows `Name <email>` assignees by their name, and comments written
  in the board are attributed to the real identity instead of "ui".

## 0.3.8 - 2026-07-13

- **0.3.7 is actually released.** The 0.3.7 release got stuck waiting for
  macOS build machines, so its binaries and npm package never went out. 0.3.8
  ships everything from 0.3.7 for Linux and Windows. Prebuilt macOS and ARM
  binaries are dropped for now; install with `cargo install wipe-cli` there.
  The docs now list exactly which platforms npm covers.

## 0.3.7 - 2026-07-13

- **Acceptance criteria.** Tickets get an acceptance-criteria list next to the
  checklist, with the same features: `wipe criteria` in the CLI, a section in
  the ticket editor and a progress badge on the card.
- **Safer ticket editing.** Edits are drafts until you press Save (or
  Cancel). Unsaved drafts survive closing the ticket or reloading the page
  without saving or logging anything. Tickets always open in preview, and long
  titles wrap instead of being cut off.
- **Livelier live updates.** New tickets type their title in, moved tickets
  float into place, and edited tickets are highlighted.
- **Windows autostart runs without a window.** The login entry now starts wipe
  hidden and removes the old console-window version.

### Fixes

- **Ticket ids can't escape the board.** Commands like
  `wipe ticket delete ../board` are refused instead of touching files outside
  the board.
- Retrying after a lost connection resumes live updates; a background refresh
  can no longer overwrite an edit you just made; browsing history handles
  stale or failed loads and boards in subfolders.
- `wipe forum show --depth` is respected with `--json`, and `wipe forum watch`
  exits when its output is closed.
- Double-clicking submit no longer creates duplicate forum posts or tickets,
  and Escape in an editor or popup no longer closes the whole ticket.

## 0.3.6 - 2026-07-11

- **Checklists.** Add an ordered list of tickable items to any ticket:
  `wipe checklist add|list|check|uncheck|toggle|edit|remove|move` (all with
  `--json`), and a checklist section with a progress bar in the ticket editor.
  Cards show a done/total badge.
- **Reloading keeps your place.** A page refresh reopens the board and view
  you had open instead of jumping back to the first board.
- **Clearer dragging.** Only the list a card would land in glows, and drop
  zones grow while dragging so fast moves don't snap back.
- **Quieter edits.** Editing an open ticket no longer flashes it as new
  activity on the board.

## 0.3.5 - 2026-07-11

- **Guided setup on first run.** The first time you run wipe after installing
  (npm or cargo), it offers to start `wipe onboard`. It asks only once, is
  skipped for `--json`, piped output and CI, and can be turned off with
  `WIPE_NO_ONBOARD_PROMPT`.
- **Docs match the CLI.** The CLI reference on wipeboard.dev now covers every
  command (onboard, identity, board, list, media, forum, scan and the full
  config), and the installation and quickstart pages explain `wipe onboard`.

## 0.3.4 - 2026-07-06

### Fixes

- **Dragging cards is reliable again.** The 0.3.3 live-update animations
  fought with drag-and-drop: the drop highlight flickered, cards animated back
  from where they started, and a dropped card could snap back to its old list.
  The gap that opens while dragging is now exactly the size of the card, with
  a dashed outline where it will land, and drops stay put. Changes made by
  agents still flash on the board.

## 0.3.3 - 2026-07-06

- **The board updates live.** An open board checks for changes every half
  second: tickets moved by agents glide between lists, changed cards flash
  briefly, and the Forum tab shows a dot when there is new activity. Nothing
  re-renders when nothing changed, and browsing history or switching boards
  isn't disturbed.
- **Update notice.** Once a day wipe checks for a newer release and prints a
  one-line notice on stderr if you are behind (never in `--json` output). Turn
  it off with `WIPE_NO_UPDATE_CHECK`.
- **The board UI is current again.** 0.3.1 and 0.3.2 shipped an outdated board
  UI inside `wipe serve`, so their UI fixes never reached you. Every release
  now includes a freshly built UI. Lists also size to their cards again
  instead of stretching to the bottom of the board.

## 0.3.2 - 2026-07-05

### Identity

- **Works with more version control systems.** wipe detects git, Plastic SCM
  (Unity VCS), Mercurial, SVN, Fossil and Jujutsu and uses the real current
  user and authors, so tickets in non-git repos are no longer attributed to
  "unknown". A slow or hung VCS tool can't block wipe.
- **Agent identities.** `wipe identity list|use|whoami|clear` lets an agent
  bind an identity to its terminal session (`$WIPE_SESSION`), and the global
  `--agentid` flag overrides it for one command.
- **A global default identity** (defaults to "human"), with an option to
  prefer it over the VCS user, in `wipe config --global`, `wipe onboard` and
  the board settings. `wipe init` no longer writes the creator's personal
  identity into the board.

### Finding boards

- **`wipe scan`** finds `.wipe` boards on disk, and `wipe serve` scans when it
  starts, so serving from any folder shows every board on the machine. The
  board UI has a Rescan button.

### Fixes

- `--agentid` no longer rewrites the board's identities on read-only commands,
  and config, registry and session files are written safely.

## 0.3.1 - 2026-07-04

- **Edits land in the board you're looking at.** Changes made while viewing a
  different board in the UI were saved to the board `wipe serve` was started
  in. They now go to the board you are viewing, the served board opens by
  default, and `wipe serve` can run outside any board as a viewer for all of
  them.
- **`wipe onboard`.** A step-by-step global setup: port, network exposure,
  auto-serve, starting at login, idle timeout, starter content, agent skill,
  theme and accent.
- **Forum polish.** Replies are drawn as a cleaner nested tree, ids are
  hidden, author names are shown, and labels can be picked from the board's
  labels when posting.
- More label colors, and lists grow with their cards instead of stretching to
  the bottom of the board.

## 0.3.0 - 2026-07-03

### Forum

- **A project forum next to the board.** Humans and agents share decisions,
  gotchas, conventions and lasting project knowledge in threaded discussions,
  stored in `.wipe/forum/` and versioned with git. Posts have readable ids
  (`F-1`, `F-1.2`, `F-1.2.1`), can carry attachments, and use the board's
  labels and identities.
- **Forum commands.** `wipe forum post|reply|show|list|search|edit|delete`,
  and `wipe forum watch`, a live stream of new posts that agents can filter by
  pattern, label, author or scope. Search matches thread titles and bodies.
- **Forum in the board.** A Board/Forum toggle opens a chat-style view with a
  thread list, threaded replies, compose, search and live updates.
- The agent skill explains how to use the forum.

### Also

- **Shell completions.** `wipe completions <bash|zsh|fish|powershell|elvish>`.
- **Safer rendering.** Markdown in tickets, comments and forum posts is
  sanitized, so HTML or scripts in a post can't run in the board.
- Forum thread ids that try to reach outside the forum folder are refused.

## 0.2.2 - 2026-07-03

- **npm install works on Windows with Git's tar on PATH.** The 0.2.1 fix was
  incomplete; extraction on Windows now always uses the built-in Windows tar.

## 0.2.1 - 2026-07-03

- **npm install on Windows with Git's tar on PATH.** When GNU tar (from Git
  for Windows) came first on PATH, `npm i -g @mflrevan/wipe` failed with
  "Cannot connect to C:" and kept the old version. (Completed in 0.2.2.)

## 0.2.0 - 2026-07-03

- **Guided `wipe init`.** An interactive setup for the board name, starter
  content (standard, lists only or empty), UI port, network exposure, auto-serve
  and agent-skill install. `--yes`, `--json` or no terminal skip the questions.
- **Install the agent skill.** `wipe skill show|install|path` writes the agent
  skill into the project's `.claude` / `.agents` folder or your user folder,
  following the convention already there.
- **Auto-serve.** With auto-serve (or `--idle`) on, `wipe serve` shuts down
  after a while with no board open, so it costs nothing when unused.
- **`wipe serve` finds a running board.** If wipe is already serving on the
  port, it points you there instead of failing.
- **Global defaults.** `wipe config --global` stores your defaults, including
  the board's accent and theme.

### Fixes

- **Dropping cards into empty lists works.** Lists now stretch to the full
  board height, so dropping anywhere in a column lands the card there instead
  of snapping back.

## 0.1.2 - 2026-07-03

- **Ticket activity.** Every ticket records who created, moved, renamed or
  edited it and who changed its priority, labels, assignees or attachments.
  The ticket view shows these next to the comments, newest first.
- **Card covers.** Cards show their first image attachment as a cover, and the
  title replaces the ticket id as the top line.
- **Richer new-card dialog.** Set priority, labels, members and attachments
  when creating a card, not just title and description.
- **Removable members.** Unassign from the member chip, or delete an agent
  identity altogether.
- Fixes for popovers being cut off or closing unexpectedly, centered avatar
  initials, and authors shown as `Name <email>` instead of their name.

## 0.1.1 - 2026-07-03

- **npm install works on Windows.** `npm i -g @mflrevan/wipe` on 0.1.0 failed
  with `spawnSync unzip ENOENT`; Windows releases now ship as `.tar.gz`.
- The README has screenshots, and wipeboard.dev is the official site and docs.

## 0.1.0 - 2026-07-03

The first release.

### CLI

- **A board in your repo.** Lists, tickets, comments, labels and priorities
  live in `.wipe/` as plain JSON, written atomically and in a stable format so
  edits to different tickets merge in git without conflicts.
- **The `wipe` command.** `init`, `status`, `board`, `list`, `ticket`,
  `comment`, `label`, `media`, `config`, `doctor` and `skill`, with a `--json`
  mode for agents, stable exit codes and authors taken from git.
- **Labels are the only categories.** Each new label gets its own color
  automatically.
- **Attachments.** Attach files to tickets, with a size limit (50 MB by
  default); files already tracked in the repo are referenced instead of
  copied.
- **An agent skill.** `wipe skill` prints the guide that teaches agents to
  work with the board.

### Board

- **`wipe serve`** opens the board in the browser (`--open`) on any port,
  with live updates when `.wipe` changes and a list of every board you have
  opened.
- **Drag-and-drop board.** Columns, cards with priority, colored labels,
  assignee avatars (agents get a bot badge) and attachment and comment counts.
  Add, rename, move and delete lists.
- **Ticket editor.** A centered ticket view with a media cover, labels,
  members, priority, an editable description, attachments that play inline
  (images, audio, video) and comments.
- **Identities.** Assign git users and agents, and give them display names.
- **History.** A git-graph view of the repo with branches and tags; click a
  commit to see the board as it was then.
- Light and dark themes with a choice of accent colors, and board settings to
  recolor or delete labels.

### Install

- Prebuilt binaries for Linux and Windows, `npm install -g @mflrevan/wipe`,
  and `cargo install wipe-cli`. macOS installs through cargo.
