//! Pinned development builds: `wipe dev link` pins a directory to a wipe binary,
//! and every `wipe` run inside it - by you or by an agent - hands the whole
//! invocation to that binary before doing anything else. This is how a local
//! sandbox runs the build under test while the installed wipe serves the rest
//! of the machine. The pins live in the user's config (`links.json`), never in
//! a repo.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use serde_json::json;
use wipe_core::config::{DevLink, DevLinks};

use crate::args::DevCmd;
use crate::output::{dim, Out};

/// Set on the delegated process: it is the pinned binary, don't delegate again.
const DELEGATED: &str = "WIPE_DELEGATED";

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The directory a raw command line runs in: the process's, or `-C/--cwd`'s.
fn target_dir(args: &[OsString]) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let s = a.to_string_lossy();
        let dir = if s == "-C" || s == "--cwd" {
            it.next().map(PathBuf::from)
        } else {
            s.strip_prefix("--cwd=").map(PathBuf::from)
        };
        if let Some(d) = dir {
            return cwd.join(d);
        }
    }
    cwd
}

/// If the command runs in a pinned directory and this isn't the pinned binary,
/// run that binary with the same arguments and return its exit code. `wipe dev`
/// itself is never handed off, so a broken build can always be unpinned.
pub fn delegate() -> Option<ExitCode> {
    if std::env::var_os(DELEGATED).is_some() {
        return None;
    }
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "dev") {
        return None;
    }
    let links = DevLinks::load();
    if links.links.is_empty() {
        return None;
    }
    let link = links.find(&canonical(&target_dir(&args)))?;
    let me = std::env::current_exe().ok().map(|p| canonical(&p));
    if me.as_deref() == Some(canonical(&link.bin).as_path()) {
        return None;
    }
    if !link.bin.is_file() {
        eprintln!(
            "wipe: {} is pinned to {}, which is missing - using this wipe instead (`wipe dev unlink` removes the pin)",
            link.dir.display(),
            link.bin.display()
        );
        return None;
    }
    let mut cmd = std::process::Command::new(&link.bin);
    // Nothing under the pinned directory finds a board above it.
    cmd.args(&args)
        .env(DELEGATED, "1")
        .env("WIPE_CEILING_DIRS", &link.dir);
    if let Some(p) = DevLinks::path() {
        cmd.env("WIPE_LINKS_FILE", p);
    }
    if let Some(c) = &link.config_dir {
        cmd.env("WIPE_CONFIG_DIR", c);
    }
    match cmd.status() {
        Ok(s) => Some(ExitCode::from(s.code().unwrap_or(1).clamp(0, 255) as u8)),
        Err(e) => {
            eprintln!("wipe: could not run the pinned {}: {e}", link.bin.display());
            Some(ExitCode::FAILURE)
        }
    }
}

/// `wipe dev ...`
pub fn run(out: &Out, cmd: DevCmd) -> Result<()> {
    match cmd {
        DevCmd::Link {
            dir,
            bin,
            config_dir,
        } => {
            let dir = canonical(&dir.unwrap_or_else(|| PathBuf::from(".")));
            if !dir.is_dir() {
                bail!("{} is not a directory", dir.display());
            }
            let bin = match bin {
                Some(b) => canonical(&b),
                None => {
                    canonical(&std::env::current_exe().context("can't tell where this wipe is")?)
                }
            };
            if !bin.is_file() {
                bail!("{} is not a file", bin.display());
            }
            let config_dir = config_dir.map(|c| {
                let _ = std::fs::create_dir_all(&c);
                canonical(&c)
            });
            let mut links = DevLinks::load();
            links.links.retain(|l| l.dir != dir);
            links.links.push(DevLink {
                dir: dir.clone(),
                bin: bin.clone(),
                config_dir: config_dir.clone(),
            });
            links.save()?;
            out.ok(
                format!("`wipe` in {} now runs {}", show(&dir), show(&bin)),
                json!({ "ok": true, "dir": dir, "bin": bin, "config_dir": config_dir }),
            );
            if !out.json && config_dir.is_some() {
                println!(
                    "  {}",
                    dim("with its own config dir (board registry, identities, update state)")
                );
            }
        }
        DevCmd::Unlink { dir } => {
            let dir = canonical(&dir.unwrap_or_else(|| PathBuf::from(".")));
            let mut links = DevLinks::load();
            let before = links.links.len();
            // The pin covering this directory, or one set on it exactly.
            let target = links.find(&dir).map(|l| l.dir.clone());
            links.links.retain(|l| Some(&l.dir) != target.as_ref());
            if links.links.len() == before {
                bail!("no pin covers {}", show(&dir));
            }
            links.save()?;
            let gone = target.unwrap_or(dir);
            out.ok(
                format!("unpinned {}", show(&gone)),
                json!({ "ok": true, "dir": gone }),
            );
        }
        DevCmd::List => {
            let links = DevLinks::load();
            let here = canonical(&std::env::current_dir().unwrap_or_default());
            let active = links.find(&here).map(|l| l.dir.clone());
            if out.json {
                out.json_value(&json!({ "links": links.links, "active": active }));
                return Ok(());
            }
            if links.links.is_empty() {
                println!("no pinned directories - `wipe dev link [DIR] --bin <wipe>` adds one");
            }
            for l in &links.links {
                let mark = if Some(&l.dir) == active.as_ref() {
                    "*"
                } else {
                    " "
                };
                println!("{mark} {}  ->  {}", show(&l.dir), show(&l.bin));
                if let Some(c) = &l.config_dir {
                    println!("    {}", dim(&format!("config: {}", show(c))));
                }
            }
            if active.is_some() {
                println!("{}", dim("* covers the current directory"));
            }
        }
    }
    Ok(())
}

fn show(p: &Path) -> String {
    wipe_core::vcs::plain(p).display().to_string()
}
