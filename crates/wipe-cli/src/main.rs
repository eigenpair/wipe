//! The `wipe` binary: parse arguments, dispatch to a command, and translate any
//! error into a clean exit code (and, in `--json` mode, a machine-readable error
//! object on stdout).

mod args;
mod autostart;
mod aware;
mod commands;
mod devlink;
mod first_run;
mod forum_cmd;
mod identity;
mod ids;
mod input;
mod onboard;
mod output;
mod safe;
mod show;
mod skills;
mod sync_cmd;
mod tray;
mod update;
mod update_check;
mod view;
mod watch;

use std::process::ExitCode;

use clap::{CommandFactory, Parser};

use args::{Cli, Command};
use output::{emit_error, Out};

/// `wipe status | head -1` closes stdout early: that is the reader's choice, not
/// an error - exit quietly like other CLIs. Any other panic becomes one line
/// that says where to report it, instead of a backtrace dump.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        if msg.contains("Broken pipe")
            || msg.contains("os error 232")
            || msg.contains("failed printing to stdout")
        {
            std::process::exit(0);
        }
        let at = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        eprintln!(
            "error: internal error: {msg}{at} - please report it at https://github.com/eigenpair/wipe/issues"
        );
    }));
}

fn main() -> ExitCode {
    install_panic_hook();
    // A directory pinned to a development build runs that build instead.
    if let Some(code) = devlink::delegate() {
        return code;
    }
    let mut cli = Cli::parse();

    // Honor `-C/--cwd` by switching directories before anything touches the board.
    if let Some(dir) = &cli.cwd {
        if let Err(e) = std::env::set_current_dir(dir) {
            emit_error(cli.json, &format!("cannot enter {}: {e}", dir.display()));
            return ExitCode::FAILURE;
        }
    }

    // Once a day, quietly note if a newer version is published (stderr only, so
    // `--json` stdout stays clean). Skipped for `completions`, whose output is
    // eval'd by the shell and should stay fast and side-effect-free.
    if !matches!(cli.command, Command::Completions { .. }) {
        if !matches!(cli.command, Command::Update(_)) {
            update_check::run(env!("CARGO_PKG_VERSION"));
        }
        // Migrate a login entry an older wipe left behind (once; stderr note).
        if let Some(note) = autostart::repair() {
            eprintln!("wipe: {note}");
        }
    }

    // Record the global --agentid override before any command resolves an author.
    identity::set_override(cli.agentid.clone());

    // On the very first interactive run of a fresh install, offer the guided global
    // setup. Skipped for the commands that either *are* that setup (`onboard`) or run
    // their own wizard (`init`), and for `completions` (shell-eval'd, must stay quiet).
    let may_offer_onboarding = !matches!(
        cli.command,
        Command::Onboard(_) | Command::Init(_) | Command::Completions { .. }
    ) && first_run::should_offer(cli.json);

    let out = Out::new(cli.json, cli.echo, cli.pretty);

    if may_offer_onboarding && first_run::offer() {
        if let Err(e) = commands::onboard(&out, args::OnboardArgs { yes: false }) {
            eprintln!("wipe: guided setup did not complete: {e:#}");
        }
    }

    // Legacy decimal boards: offer the hex-id translation once; then accept any
    // spelling of a ticket id (`t2a`, `T-02A`, a pre-translation `T-42`).
    ids::resume_interrupted();
    ids::offer_translation(cli.json, &cli.command);
    ids::canonicalize(&mut cli.command);

    let write = is_write(&cli.command);
    // Every write needs a chosen identity - there is no default to fall back to.
    // Refuse up front, before anything is read or written, with guidance.
    let mut _lock = None;
    if write {
        let Some(who) = identity::resolve_opt(actor_override(&cli.command)) else {
            emit_error(cli.json, &identity::missing_identity_message());
            return ExitCode::FAILURE;
        };
        // Serialize with every other writer of this board (other agents, the UI).
        if let Ok(s) = wipe_core::Store::discover(".") {
            match s.lock() {
                Ok(l) => _lock = Some(l),
                Err(e) => {
                    emit_error(cli.json, &format!("cannot take the board write lock: {e}"));
                    return ExitCode::FAILURE;
                }
            }
        }
        // An agent named by --agentid / $WIPE_AGENT shows up in the board's
        // identity list like any other author (best-effort, insert-only).
        if cli.agentid.is_some() || identity::agent_env().is_some() {
            identity::ensure_registered(&who, None, true);
        }
    }

    let sync_lines = aware::sync_lines(&cli.command);
    let aware_mode = aware::mode(&cli.command);
    let finish = aware::finish(&cli.command);
    let actor = actor_override(&cli.command).map(str::to_string);
    let result = dispatch(&out, cli.command);
    // What others changed on the board since this identity's last command
    // (stderr; after the output, whether the command succeeded or not).
    let report_updates = |finished| aware::after_command(aware_mode, finished, actor.as_deref());

    match result {
        Ok(()) => {
            report_updates(finish);
            // Uncommitted / incoming board changes, so they're never forgotten.
            if sync_lines {
                sync_cmd::after_command();
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            match e.downcast_ref::<sync_cmd::Conflicts>() {
                Some(c) if cli.json => out.json_value(&sync_cmd::conflicts_json(c)),
                _ => emit_error(cli.json, &format!("{e:#}")),
            }
            report_updates(aware::Finish::No);
            ExitCode::FAILURE
        }
    }
}

/// Whether a command writes to the board (and so needs an identity and takes the
/// write lock). Reads - including
/// identity-scoped ones like `inbox` - are never gated.
fn is_write(c: &Command) -> bool {
    use args::*;
    match c {
        Command::Board(b) => !matches!(b, BoardCmd::Show),
        Command::List(l) => !matches!(l, ListCmd::Show),
        Command::Ticket(t) => !matches!(t, TicketCmd::Show(_) | TicketCmd::List(_)),
        Command::Comment(x) => !matches!(x, CommentCmd::List { .. }),
        Command::Checklist(x) | Command::Criteria(x) => !matches!(x, ChecklistCmd::List { .. }),
        Command::Label(x) => !matches!(x, LabelCmd::List),
        Command::Media(x) => !matches!(x, MediaCmd::List { .. }),
        Command::Forum(x) => matches!(
            x,
            ForumCmd::Post(_)
                | ForumCmd::Reply(_)
                | ForumCmd::Edit { .. }
                | ForumCmd::Delete { .. }
                | ForumCmd::Pin { .. }
                | ForumCmd::Unpin { .. }
        ),
        Command::Subscribe(_)
        | Command::Unsubscribe(_)
        | Command::Commit(_)
        | Command::Sync(args::SyncArgs { check: false, .. })
        | Command::Restore(_)
        | Command::Migrate(_) => true,
        Command::Trash { cmd } => !matches!(cmd, TrashCmd::List),
        Command::Config { global, cmd } => !global && matches!(cmd, ConfigCmd::Set { .. }),
        _ => false,
    }
}

/// The per-command *actor* override, if the command carries one. Reattribution
/// targets (e.g. `ticket edit --author`, `comment reattribute --to`) are NOT actor
/// overrides - those writes are still performed by the session identity.
fn actor_override(c: &Command) -> Option<&str> {
    use args::{CommentCmd, ForumCmd};
    match c {
        Command::Comment(CommentCmd::Add { author, .. }) => author.as_deref(),
        Command::Forum(ForumCmd::Post(a)) => a.author.as_deref(),
        Command::Forum(ForumCmd::Reply(a)) => a.author.as_deref(),
        Command::Subscribe(a) | Command::Unsubscribe(a) => a.author.as_deref(),
        Command::Commit(a) => a.author.as_deref(),
        _ => None,
    }
}

fn dispatch(out: &Out, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Init(a) => commands::init(out, a),
        Command::Onboard(a) => commands::onboard(out, a),
        Command::Identity(c) => commands::identity(out, c),
        Command::Scan(a) => commands::scan(out, a),
        Command::Status(a) => commands::status(out, a),
        Command::Board(c) => commands::board(out, c),
        Command::List(c) => commands::list(out, c),
        Command::Ticket(c) => commands::ticket(out, c),
        Command::Comment(c) => commands::comment(out, c),
        Command::Checklist(c) => commands::checklist(out, c),
        Command::Criteria(c) => commands::criteria(out, c),
        Command::Label(c) => commands::label(out, c),
        Command::Media(c) => commands::media(out, c),
        Command::Forum(c) => forum_cmd::run(out, c),
        Command::Serve(a) => commands::serve(out, a),
        Command::Tray(a) => tray::run(a),
        Command::Config { global, cmd } => commands::config(out, global, cmd),
        Command::Subscribe(a) => commands::subscribe(out, a, false),
        Command::Unsubscribe(a) => commands::subscribe(out, a, true),
        Command::Subscriptions { author } => commands::subscriptions(out, author),
        Command::Inbox(a) => commands::inbox(out, a),
        Command::Search(a) => commands::search(out, a),
        Command::Watch(a) => watch::run(out, a),
        Command::Trash { cmd } => commands::trash(out, cmd),
        Command::Commit(a) => sync_cmd::commit(out, a),
        Command::Sync(a) => sync_cmd::sync(out, a),
        Command::Migrate(a) => sync_cmd::migrate(out, a),
        Command::Log(a) => sync_cmd::log(out, a),
        Command::Restore(a) => sync_cmd::restore(out, a),
        Command::Doctor => commands::doctor(out),
        Command::Update(a) => update::run(out, a),
        Command::Dev(c) => devlink::run(out, c),
        Command::Skill { cmd } => commands::skill(out, cmd),
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "wipe", &mut std::io::stdout());
            Ok(())
        }
    }
}
