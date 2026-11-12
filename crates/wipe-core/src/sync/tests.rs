//! Two clones of one repo, sharing a board through the wipe branch on a bare
//! remote.

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;

use super::*;
use crate::model::Ticket;
use crate::ops::{self, NewTicket, TicketPatch};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn identity(dir: &Path) {
    git(dir, &["config", "user.name", "Tester"]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

struct World {
    _tmp: tempfile::TempDir,
    remote: PathBuf,
    a: Store,
}

/// A repo with a pre-1.0 board tracked on `main`, pushed to a bare remote, then
/// migrated to the wipe branch.
fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let a = tmp.path().join("a");
    std::fs::create_dir_all(&a).unwrap();
    git(
        tmp.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    git(&a, &["init", "-q", "-b", "main"]);
    identity(&a);
    git(&a, &["remote", "add", "origin", remote.to_str().unwrap()]);
    std::fs::write(a.join("README.md"), "hi\n").unwrap();
    let store = Store::init(&a, "Shared", Utc::now()).unwrap();
    git(&a, &["add", "."]);
    git(&a, &["commit", "-q", "-m", "init"]);
    git(&a, &["push", "-q", "origin", "main"]);
    assert_eq!(status(&store).unwrap().mode, Mode::Unmigrated);

    let m = migrate(
        &store,
        "Ada <ada@example.com>",
        &MigrateOpts {
            publish: true,
            code_commit: true,
            resolutions: Resolutions::default(),
        },
    )
    .unwrap();
    assert!(!m.adopted);
    assert_eq!(m.published.as_deref(), Some("origin"));
    assert!(m.code_commit.is_some(), "untracking committed");
    assert_eq!(
        git(&a, &["ls-files", ".wipe"]),
        "",
        ".wipe untracked on main"
    );
    assert!(std::fs::read_to_string(a.join(".gitignore"))
        .unwrap()
        .contains("/.wipe/"));
    git(&a, &["push", "-q", "origin", "main"]);
    let st = status(&store).unwrap();
    assert_eq!(st.mode, Mode::Branch);
    assert!(st.local.is_empty() && st.incoming.is_empty(), "{st:?}");
    World {
        _tmp: tmp,
        remote,
        a: Store::open(&a).unwrap(),
    }
}

/// A fresh clone: its `.wipe/` is checked out of the branch on first use.
fn clone(w: &World, name: &str) -> Store {
    let dir = w.remote.parent().unwrap().join(name);
    git(
        w.remote.parent().unwrap(),
        &[
            "clone",
            "-q",
            w.remote.to_str().unwrap(),
            dir.to_str().unwrap(),
        ],
    );
    identity(&dir);
    assert!(!dir.join(".wipe").exists(), "not tracked on main");
    let s = Store::discover(&dir).unwrap();
    assert!(s.wipe_dir().join("board.json").is_file());
    s
}

fn create(s: &Store, title: &str, list: &str) -> Ticket {
    ops::create_ticket(
        s,
        NewTicket {
            title: title.into(),
            list: Some(list.into()),
            ..Default::default()
        },
        "tester",
        Utc::now(),
    )
    .unwrap()
}

fn commit_ok(s: &Store, msg: Option<&str>) -> Committed {
    let c = commit(
        s,
        msg,
        None,
        "Tester <t@example.com>",
        &Resolutions::default(),
    )
    .unwrap();
    assert!(c.conflicts.is_empty(), "{:?}", c.conflicts);
    c
}

fn sync_ok(s: &Store) -> Synced {
    let r = sync(s, true, &Resolutions::default()).unwrap();
    assert!(r.conflicts.is_empty(), "{:?}", r.conflicts);
    r
}

#[test]
fn local_changes_commit_publish_and_arrive_elsewhere() {
    let w = world();
    let b = clone(&w, "b");

    let t = create(&w.a, "From A", "todo");
    let st = status(&w.a).unwrap();
    assert_eq!(st.local.len(), 1);
    assert_eq!(
        (st.local[0].kind.as_str(), st.local[0].id.as_str()),
        ("created", t.id.as_str())
    );

    // The status is cached between commands: it must read back whole.
    let again: Status = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
    assert_eq!(again.local, st.local);
    assert_eq!(status(&w.a).unwrap().local, st.local, "from the cache");

    let c = commit_ok(&w.a, None);
    assert!(c.commit.is_some());
    assert!(
        c.message.starts_with("board: T-001 created"),
        "{}",
        c.message
    );
    assert!(status(&w.a).unwrap().local.is_empty());
    // Only the board is on the branch.
    let files = git(w.a.root(), &["ls-tree", "-r", "--name-only", "wipe"]);
    assert!(files.lines().all(|l| l.starts_with(".wipe/")), "{files}");
    assert!(!files.contains(".cache"));

    // B sees it as incoming after a fetch, and applies it.
    let st = refresh(&b).unwrap();
    assert_eq!(st.incoming.len(), 1, "{st:?}");
    assert_eq!(st.commits.len(), 1);
    let r = sync_ok(&b);
    assert_eq!(r.applied.len(), 1);
    assert_eq!(b.load_ticket("T-001").unwrap().title, "From A");
    let st = status(&b).unwrap();
    assert!(st.local.is_empty() && st.incoming.is_empty(), "{st:?}");
}

#[test]
fn both_sides_creating_tickets_renumbers_ours() {
    let w = world();
    let b = clone(&w, "b");
    create(&w.a, "A's ticket", "todo");
    let bt = create(&b, "B's ticket", "in-progress");
    assert_eq!(bt.id, "T-001", "same counter on both sides");
    ops::add_comment(&b, "T-001", "bob", "see T-001 for details", Utc::now()).unwrap();
    commit_ok(&w.a, Some("A work"));

    // B commits: A's T-001 comes in, B's becomes T-002 with its references.
    let c = commit_ok(&b, None);
    assert_eq!(
        c.renumbered,
        vec![("T-001".to_string(), "T-002".to_string())]
    );
    assert_eq!(c.merged.len(), 1);
    assert_eq!(b.load_ticket("T-001").unwrap().title, "A's ticket");
    let t2 = b.load_ticket("T-002").unwrap();
    assert_eq!(t2.title, "B's ticket");
    assert_eq!(t2.comments[0].body, "see T-002 for details");
    let board = b.load_board().unwrap();
    assert_eq!(board.locate_card("T-001").unwrap().0, "todo");
    assert_eq!(board.locate_card("T-002").unwrap().0, "in-progress");
    assert_eq!(board.next_ticket, 3);

    sync_ok(&w.a);
    assert_eq!(w.a.load_ticket("T-002").unwrap().title, "B's ticket");
    assert!(status(&w.a).unwrap().local.is_empty());
}

#[test]
fn field_level_merge_and_conflicts() {
    let w = world();
    create(&w.a, "Shared", "todo");
    ops::update_ticket(
        &w.a,
        "T-001",
        TicketPatch {
            body: Some("line one\nline two\nline three".into()),
            ..Default::default()
        },
        "tester",
        Utc::now(),
    )
    .unwrap();
    commit_ok(&w.a, None);
    let b = clone(&w, "b");

    // Different fields and different lines of the body: merges cleanly.
    let patch = |s: &Store, p: TicketPatch| {
        ops::update_ticket(s, "T-001", p, "tester", Utc::now()).unwrap();
    };
    patch(
        &w.a,
        TicketPatch {
            body: Some("line one (a)\nline two\nline three".into()),
            priority: Some(Some("high".into())),
            ..Default::default()
        },
    );
    patch(
        &b,
        TicketPatch {
            body: Some("line one\nline two\nline three (b)".into()),
            labels: Some(vec!["agent".into()]),
            ..Default::default()
        },
    );
    ops::add_comment(&w.a, "T-001", "alice", "from a", Utc::now()).unwrap();
    ops::add_comment(&b, "T-001", "bob", "from b", Utc::now()).unwrap();
    commit_ok(&w.a, None);
    commit_ok(&b, None);
    let t = b.load_ticket("T-001").unwrap();
    assert_eq!(t.body, "line one (a)\nline two\nline three (b)");
    assert_eq!(t.priority.as_deref(), Some("high"));
    assert_eq!(t.labels, ["agent"]);
    let comments: Vec<(&str, &str)> = t
        .comments
        .iter()
        .map(|c| (c.id.as_str(), c.body.as_str()))
        .collect();
    assert_eq!(
        comments,
        [("c-1", "from a"), ("c-2", "from b")],
        "b's comment renumbered"
    );
    assert_eq!(t.next_comment, 3);

    // Same field, two values: a conflict, nothing written until resolved.
    sync_ok(&w.a);
    patch(
        &w.a,
        TicketPatch {
            title: Some("Title A".into()),
            ..Default::default()
        },
    );
    patch(
        &b,
        TicketPatch {
            title: Some("Title B".into()),
            ..Default::default()
        },
    );
    ops::move_ticket(&w.a, "T-001", "in-progress", None, "alice", Utc::now()).unwrap();
    ops::move_ticket(&b, "T-001", "done", None, "bob", Utc::now()).unwrap();
    commit_ok(&w.a, None);
    let before = std::fs::read(b.wipe_dir().join("tickets/T-001.json")).unwrap();
    let c = commit(
        &b,
        None,
        None,
        "Bob <b@example.com>",
        &Resolutions::default(),
    )
    .unwrap();
    let ids: Vec<&str> = c.conflicts.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"tickets/T-001.json#/title"), "{ids:?}");
    assert!(ids.contains(&"board.json#/cards[T-001]"), "{ids:?}");
    assert!(c.commit.is_none());
    assert_eq!(
        std::fs::read(b.wipe_dir().join("tickets/T-001.json")).unwrap(),
        before,
        "untouched"
    );

    // Keep ours for the title, theirs for the list.
    let mut res = Resolutions::default();
    res.picks
        .insert("tickets/T-001.json#/title".into(), Side::Ours);
    res.picks
        .insert("board.json#/cards[T-001]".into(), Side::Theirs);
    let c = commit(&b, None, None, "Bob <b@example.com>", &res).unwrap();
    assert!(c.conflicts.is_empty(), "{:?}", c.conflicts);
    assert!(c.commit.is_some());
    assert_eq!(b.load_ticket("T-001").unwrap().title, "Title B");
    assert_eq!(
        b.load_board().unwrap().locate_card("T-001").unwrap().0,
        "in-progress"
    );
    sync_ok(&w.a);
    assert_eq!(w.a.load_ticket("T-001").unwrap().title, "Title B");
}

#[test]
fn delete_against_edit_is_a_conflict() {
    let w = world();
    create(&w.a, "Doomed", "todo");
    commit_ok(&w.a, None);
    let b = clone(&w, "b");
    ops::delete_ticket(&w.a, "T-001", Utc::now()).unwrap();
    commit_ok(&w.a, None);
    ops::add_comment(&b, "T-001", "bob", "still needed", Utc::now()).unwrap();
    let r = sync(&b, true, &Resolutions::default()).unwrap();
    assert_eq!(r.conflicts.len(), 1, "{:?}", r.conflicts);
    assert_eq!(r.conflicts[0].id, "tickets/T-001.json#");
    // Keep it: the ticket and its card come back.
    let res = Resolutions {
        default: Some(Side::Ours),
        ..Default::default()
    };
    let r = sync(&b, true, &res).unwrap();
    assert!(r.conflicts.is_empty());
    assert!(b.load_board().unwrap().locate_card("T-001").is_some());
    let st = status(&b).unwrap();
    assert!(
        st.local
            .iter()
            .any(|c| c.id == "T-001" && c.kind == "created"),
        "{st:?}"
    );
}

#[test]
fn worktrees_share_one_working_copy_and_old_branches_are_flagged() {
    let w = world();
    let a = w.a.root().to_path_buf();
    let wt = a.parent().unwrap().join("wt");
    git(
        &a,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );
    let s = Store::discover(&wt).unwrap();
    assert_eq!(
        std::fs::canonicalize(s.root()).unwrap(),
        std::fs::canonicalize(&a).unwrap(),
        "the main worktree's board"
    );

    // A branch from before the move still tracks .wipe/.
    git(&a, &["checkout", "-q", "-b", "old", "HEAD~1"]);
    let st = status(&w.a).unwrap();
    assert_eq!(st.mode, Mode::Tracked);
    assert!(commit(&w.a, None, None, "x", &Resolutions::default()).is_err());
    let m = migrate(
        &w.a,
        "Ada <ada@example.com>",
        &MigrateOpts {
            publish: true,
            code_commit: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(m.adopted && m.conflicts.is_empty());
    assert_eq!(status(&w.a).unwrap().mode, Mode::Branch);
}

#[test]
fn a_new_board_goes_straight_onto_the_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("p");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    identity(&dir);
    let s = Store::init(&dir, "Fresh", Utc::now()).unwrap();
    adopt_new_board(&s, "Ada <ada@example.com>").unwrap();
    let st = status(&s).unwrap();
    assert_eq!(st.mode, Mode::Branch);
    assert!(st.local.is_empty());
    create(&s, "First", "todo");
    let c = commit_ok(&s, Some("first ticket"));
    assert_eq!(c.message, "first ticket");
    assert!(c.remote.is_none());
    assert_eq!(
        git(&dir, &["log", "-1", "--format=%s", "wipe"]),
        "first ticket"
    );
    assert_eq!(git(&dir, &["log", "-1", "--format=%an", "wipe"]), "Tester");
}

#[test]
fn forum_threads_and_replies_from_both_sides_are_kept() {
    use crate::forum::{self, NewReply, NewThread};
    let w = world();
    // A board still on the thread forum (before 1.1).
    let mut board = w.a.load_board().unwrap();
    board.forum = crate::model::ForumFormat::Threads;
    w.a.save_board(&board).unwrap();
    let thread = |s: &Store, title: &str| {
        forum::create_thread(
            s,
            NewThread {
                title: title.into(),
                body: format!("{title} body"),
                labels: vec![],
                refs: vec![],
                attachments: vec![],
            },
            "tester",
            Utc::now(),
        )
        .unwrap()
    };
    let reply = |s: &Store, parent: &str, body: &str| {
        forum::reply(
            s,
            parent,
            NewReply {
                summary: body.into(),
                body: body.into(),
                labels: vec![],
                refs: vec![],
                attachments: vec![],
            },
            "tester",
            Utc::now(),
        )
        .unwrap()
    };
    thread(&w.a, "Shared");
    commit_ok(&w.a, None);
    let b = clone(&w, "b");

    // Both start F-2 and both reply F-1.1.
    assert_eq!(thread(&w.a, "A's thread"), "F-2");
    assert_eq!(thread(&b, "B's thread"), "F-2");
    assert_eq!(reply(&w.a, "F-1", "a says"), "F-1.1");
    assert_eq!(reply(&b, "F-1", "b says"), "F-1.1");
    commit_ok(&w.a, None);
    let c = commit_ok(&b, None);
    assert!(
        c.renumbered
            .contains(&("F-2".to_string(), "F-3".to_string())),
        "{:?}",
        c.renumbered
    );

    let titles: Vec<String> = b
        .load_all_threads()
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(titles, ["Shared", "A's thread", "B's thread"]);
    let shared = b.load_thread("F-1").unwrap();
    let replies: Vec<(&str, &str)> = shared
        .root
        .replies
        .iter()
        .map(|r| (r.id.as_str(), r.body.as_str()))
        .collect();
    assert_eq!(replies, [("F-1.1", "a says"), ("F-1.2", "b says")]);
    assert_eq!(
        b.load_thread("F-3").unwrap().root.id,
        "F-3",
        "post ids follow the thread"
    );
}

#[test]
fn forum_posts_from_both_sides_are_kept_and_their_tags_follow() {
    use crate::forum::{self, NewPost};
    let w = world();
    let post = |s: &Store, summary: &str, body: &str| {
        let spec = NewPost {
            summary: summary.into(),
            body: body.into(),
            ..Default::default()
        };
        forum::create_post(s, spec, "tester", Utc::now()).unwrap()
    };
    post(&w.a, "Shared", "the topic");
    commit_ok(&w.a, None);
    let b = clone(&w, "b");

    // Both reply with F-002; B replies to its own reply too.
    assert_eq!(post(&w.a, "A agrees", "@F-001 yes"), "F-002");
    assert_eq!(post(&b, "B disagrees", "@F-001 no"), "F-002");
    assert_eq!(post(&b, "B again", "@F-002 because"), "F-003");
    commit_ok(&w.a, None);
    let c = commit_ok(&b, None);
    assert!(
        c.renumbered
            .contains(&("F-002".to_string(), "F-004".to_string())),
        "{:?}",
        c.renumbered
    );
    let get = |id: &str| forum::get_post(&b, id).unwrap();
    assert_eq!(get("F-002").summary, "A agrees");
    assert_eq!(get("F-004").summary, "B disagrees");
    // B's own tag followed its renumbered post.
    let again = forum::index(&b)
        .unwrap()
        .into_iter()
        .find(|p| p.summary == "B again")
        .unwrap();
    assert!(again.body.starts_with("@F-004 "), "{}", again.body);
    assert_eq!(again.thread_id, "F-001");
}

#[test]
fn two_peoples_agents_with_one_name_stay_two_identities() {
    use crate::ops::{self, NewTicket};
    let w = world();
    commit_ok(&w.a, None);
    let b = clone(&w, "b");
    // Both people's agents call themselves `claude`, unaware of each other.
    ops::claim_agent(&w.a, "claude", None, Some("ada@x")).unwrap();
    ops::claim_agent(&b, "claude", None, Some("bob@x")).unwrap();
    let spec = NewTicket {
        title: "by bob's claude, cc @claude".into(),
        list: Some("todo".into()),
        assignees: vec!["claude".into()],
        ..Default::default()
    };
    let t = ops::create_ticket(&b, spec, "claude", Utc::now()).unwrap();
    commit_ok(&w.a, None);
    let c = commit_ok(&b, None);
    assert!(
        c.renumbered
            .contains(&("claude".to_string(), "claude02".to_string())),
        "{:?}",
        c.renumbered
    );
    let ids = ops::list_identities(&b).unwrap();
    let owner = |id: &str| {
        ids.iter()
            .find(|i| i.id == id)
            .and_then(|i| i.owner.clone())
    };
    assert_eq!(
        owner("claude").as_deref(),
        Some("ada@x"),
        "the incoming one keeps the id"
    );
    assert_eq!(owner("claude02").as_deref(), Some("bob@x"));
    let t = b.load_ticket(&t.id).unwrap();
    assert_eq!(t.assignees, ["claude02"]);
    assert_eq!(t.title, "by bob's claude, cc @claude02");
    assert_eq!(t.creator(), Some("claude02"));
    // Bob's agent asking for `claude` now acts as its own.
    assert_eq!(ops::agent_for(&b, "claude", Some("bob@x")), "claude02");
}

#[test]
fn logs_merge_in_time_order_and_rewritten_history_merges_from_your_board() {
    use crate::ops;
    let w = world();
    let t = create(&w.a, "Logged", "todo").id;
    commit_ok(&w.a, None);
    let b = clone(&w, "b");
    // A comments first, B a moment later; A publishes first.
    ops::add_comment(&w.a, &t, "ada", "first", Utc::now()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    ops::add_comment(&b, &t, "bob", "second", Utc::now()).unwrap();
    commit_ok(&w.a, None);
    commit_ok(&b, None);
    let bodies: Vec<String> = b
        .load_ticket(&t)
        .unwrap()
        .comments
        .into_iter()
        .map(|c| c.body)
        .collect();
    assert_eq!(
        bodies,
        ["first", "second"],
        "in the order they were written"
    );

    // The branch is replaced by unrelated history with the same board: an
    // edit made meanwhile merges cleanly instead of clashing with it.
    let dir = w.a.root().to_path_buf();
    let up = git(&dir, &["rev-parse", "origin/wipe^{tree}"]);
    let root = git(&dir, &["commit-tree", up.trim(), "-m", "rewritten"]);
    let spec = format!("{}:refs/heads/wipe", root.trim());
    git(&dir, &["push", "-q", "-f", "origin", &spec]);
    let mut tk = b.load_ticket(&t).unwrap();
    tk.title = "edited meanwhile".into();
    b.save_ticket(&tk).unwrap();
    commit_ok(&b, None);
    assert_eq!(b.load_ticket(&t).unwrap().title, "edited meanwhile");
}

#[test]
fn a_clone_inside_another_board_gets_its_own_from_the_branch() {
    let w = world();
    // A directory with a board of its own, holding a fresh clone.
    let outer = w.remote.parent().unwrap().join("outer");
    std::fs::create_dir_all(&outer).unwrap();
    Store::init(&outer, "Outer", Utc::now()).unwrap();
    git(
        &outer,
        &["clone", "-q", w.remote.to_str().unwrap(), "inner"],
    );
    let s = Store::discover(outer.join("inner")).unwrap();
    assert_eq!(
        s.load_board().unwrap().name,
        "Shared",
        "the clone's board, not the outer one"
    );
    assert_eq!(status(&s).unwrap().mode, Mode::Branch);
}

fn ticket(id: &str) -> Entity {
    Entity {
        scope: "ticket".into(),
        id: id.into(),
        fields: vec![],
    }
}

#[test]
fn taking_incoming_changes_one_at_a_time() {
    let w = world();
    create(&w.a, "Shared", "todo");
    commit_ok(&w.a, None);
    let b = clone(&w, "b");

    // Alice: a new ticket and a retitle. Bob: a new ticket of his own (same id).
    create(&w.a, "From Alice", "todo");
    ops::update_ticket(
        &w.a,
        "T-001",
        TicketPatch {
            title: Some("Shared, renamed".into()),
            ..Default::default()
        },
        "alice",
        Utc::now(),
    )
    .unwrap();
    commit_ok(&w.a, None);
    let mine = create(&b, "From Bob", "in-progress");
    assert_eq!(mine.id, "T-002");
    refresh(&b).unwrap();

    // Take only Alice's T-002: Bob's makes room, the retitle stays incoming.
    let t = take(
        &b,
        &Source::Incoming,
        &[ticket("T-002")],
        &Resolutions::default(),
        false,
    )
    .unwrap();
    assert!(t.conflicts.is_empty());
    assert_eq!(
        t.renumbered,
        vec![("T-002".to_string(), "T-003".to_string())]
    );
    assert!(!t.synced);
    assert_eq!(b.load_ticket("T-002").unwrap().title, "From Alice");
    assert_eq!(b.load_ticket("T-003").unwrap().title, "From Bob");
    let st = status(&b).unwrap();
    let ids = |cs: &[Change]| {
        cs.iter()
            .map(|c| format!("{} {}", c.id, c.kind))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&st.incoming),
        ["T-001 renamed"],
        "taken changes are no longer incoming"
    );
    assert!(ids(&st.local).contains(&"T-003 created".to_string()));
    assert!(
        !ids(&st.local).iter().any(|c| c.starts_with("T-002")),
        "nor local: {:?}",
        st.local
    );

    // Edit the taken ticket, take the rest: the board moves up to the branch.
    ops::update_ticket(
        &b,
        "T-002",
        TicketPatch {
            body: Some("bob's notes".into()),
            ..Default::default()
        },
        "bob",
        Utc::now(),
    )
    .unwrap();
    let t = take(
        &b,
        &Source::Incoming,
        &[ticket("T-001")],
        &Resolutions::default(),
        false,
    )
    .unwrap();
    assert!(t.synced);
    let st = status(&b).unwrap();
    assert!(st.incoming.is_empty());
    assert_eq!(st.base, st.upstream);
    // Committing doesn't duplicate the taken ticket.
    commit_ok(&b, None);
    let tickets: Vec<String> = b
        .load_all_tickets()
        .unwrap()
        .into_iter()
        .map(|t| format!("{} {}", t.id, t.title))
        .collect();
    assert_eq!(
        tickets,
        [
            "T-001 Shared, renamed",
            "T-002 From Alice",
            "T-003 From Bob"
        ]
    );
    assert_eq!(b.load_ticket("T-002").unwrap().body, "bob's notes");
}

#[test]
fn taking_a_ticket_back_from_a_past_commit() {
    let w = world();
    create(&w.a, "Original", "todo");
    let first = commit_ok(&w.a, None).commit.unwrap();
    ops::update_ticket(
        &w.a,
        "T-001",
        TicketPatch {
            title: Some("Changed".into()),
            ..Default::default()
        },
        "alice",
        Utc::now(),
    )
    .unwrap();
    ops::move_ticket(&w.a, "T-001", "done", None, "alice", Utc::now()).unwrap();
    commit_ok(&w.a, None);

    let at_first = at(&w.a, &first).unwrap();
    assert_eq!(
        at_first.lists.iter().find(|(l, _)| l == "todo").unwrap().1[0].title,
        "Original"
    );
    assert!(at_first
        .changes
        .iter()
        .any(|c| c.id == "T-001" && c.kind == "created"));
    let entries = log(&w.a, 10).unwrap();
    assert_eq!(entries[0].place, "base");

    // Uncommitted work on it is never discarded silently.
    let mut t1 = w.a.load_ticket("T-001").unwrap();
    t1.body = "unsaved notes".into();
    w.a.save_ticket(&t1).unwrap();
    let refused = take(
        &w.a,
        &Source::Commit(first.clone()),
        &[ticket("T-001")],
        &Resolutions::default(),
        false,
    )
    .unwrap();
    assert_eq!(refused.discards, ["T-001"]);
    assert_eq!(
        w.a.load_ticket("T-001").unwrap().body,
        "unsaved notes",
        "nothing changed"
    );

    // Asked to: the old version replaces it, unsaved notes and all.
    take(
        &w.a,
        &Source::Commit(first),
        &[ticket("T-001")],
        &Resolutions::default(),
        true,
    )
    .unwrap();
    assert_eq!(w.a.load_ticket("T-001").unwrap().title, "Original");
    assert_eq!(
        w.a.load_board().unwrap().locate_card("T-001").unwrap().0,
        "todo"
    );
    let st = status(&w.a).unwrap();
    assert!(
        st.local
            .iter()
            .any(|c| c.id == "T-001" && c.kind == "moved"),
        "{:?}",
        st.local
    );
}
