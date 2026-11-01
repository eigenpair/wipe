#!/usr/bin/env node
// A local sandbox for testing wipe before a release - untracked, under
// tests/sandbox/ (gitignored):
//
//   tests/sandbox/
//     bin/wipe[.exe]   the build under test (a copy, so a running `wipe serve`
//                      survives rebuilds and a broken `cargo build` can't break it)
//     .config/         its own config dir: board registry, identities, sessions
//     scenario/        a ready-made team: remote.git (bare) + clones alice/, bob/
//     activate.ps1|.sh put bin/ first on PATH for one shell
//
// `wipe` run anywhere under tests/sandbox/ - by you or by an agent - uses the
// sandbox build: the directory is pinned with `wipe dev link`, and an installed
// wipe (1.0+) hands any command run there to the pinned binary. (Until the
// installed wipe is 1.0+, `activate` the sandbox in a shell instead.)
//
//   node scripts/sandbox.mjs setup     build, install into the sandbox, pin it,
//                                      create the scenario (keeps an existing one)
//   node scripts/sandbox.mjs update    rebuild + reinstall the binary (--ui: also
//                                      rebuild the embedded board UI first)
//   node scripts/sandbox.mjs reset     recreate the scenario from scratch
//   node scripts/sandbox.mjs status    what's installed and pinned
//   node scripts/sandbox.mjs remove    unpin it and delete what setup made
import { existsSync, mkdirSync, readdirSync, renameSync, rmSync, copyFileSync, writeFileSync } from "node:fs";
import { execFileSync, execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { embedUi } from "./embed-ui.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const box = join(root, "tests", "sandbox");
const binDir = join(box, "bin");
const exe = process.platform === "win32" ? "wipe.exe" : "wipe";
const bin = join(binDir, exe);
const configDir = join(box, ".config");
const scenario = join(box, "scenario");

const [cmd = "status", ...flags] = process.argv.slice(2);

function run(file, args, opts = {}) {
  return execFileSync(file, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...opts });
}

/** Run the sandbox build as it runs for anyone working in the sandbox. */
function wipe(args, cwd, agent) {
  return run(bin, args, {
    cwd,
    env: { ...process.env, WIPE_CONFIG_DIR: configDir, WIPE_CEILING_DIRS: box, WIPE_AGENT: agent, WIPE_NO_UPDATES: "1" },
  });
}

function git(args, cwd) {
  return run("git", args, { cwd });
}

function build(ui) {
  if (ui) embedUi(root);
  console.log("Building wipe (release)…");
  execSync("cargo build --release -p wipe-cli", { cwd: root, stdio: "inherit" });
}

/** Copy the fresh build in. A running sandbox wipe (serve, tray) holds the old
 *  binary open on Windows: it can be renamed but not overwritten, so it moves
 *  aside first and is cleaned up once nothing runs it. */
function install() {
  mkdirSync(binDir, { recursive: true });
  const built = join(root, "target", "release", exe);
  if (!existsSync(built)) throw new Error(`no build at ${built}`);
  const old = join(binDir, ".old");
  if (existsSync(bin)) {
    mkdirSync(old, { recursive: true });
    renameSync(bin, join(old, `${Date.now()}-${exe}`));
  }
  copyFileSync(built, bin);
  if (existsSync(old)) {
    for (const f of readdirSync(old)) {
      try {
        rmSync(join(old, f));
      } catch {
        /* still running; removed on a later update */
      }
    }
  }
  console.log(`installed ${wipe(["--version"], box).trim()} -> ${bin}`);
}

/** Its own config: no onboarding prompt, and `wipe serve` lists sandbox boards only. */
function seedConfig() {
  mkdirSync(configDir, { recursive: true });
  const cfg = join(configDir, "config.json");
  if (!existsSync(cfg)) {
    writeFileSync(cfg, JSON.stringify({ default_identity: "human", scan_roots: [box] }, null, 2) + "\n");
  }
  writeFileSync(join(configDir, "first-run.json"), '{ "offered": true }\n');
}

/** Pin tests/sandbox/ to the sandbox build (the pin lives in your own config). */
function pin() {
  // Run without WIPE_CONFIG_DIR: the pin goes into the real links.json, where an
  // installed wipe looks for it.
  const env = { ...process.env };
  delete env.WIPE_CONFIG_DIR;
  console.log(run(bin, ["dev", "link", box, "--bin", bin, "--config-dir", configDir], { env }).trim());
}

function activateScripts() {
  writeFileSync(
    join(box, "activate.ps1"),
    `# Use the sandbox wipe in this PowerShell session: . tests/sandbox/activate.ps1\n` +
      `$env:PATH = "${binDir};" + $env:PATH\n$env:WIPE_CONFIG_DIR = "${configDir}"\n$env:WIPE_CEILING_DIRS = "${box}"\n` +
      `Write-Host "wipe -> ${bin}"\n`
  );
  writeFileSync(
    join(box, "activate.sh"),
    `# Use the sandbox wipe in this shell: source tests/sandbox/activate.sh\n` +
      `export PATH="${binDir.replace(/\\/g, "/")}:$PATH"\nexport WIPE_CONFIG_DIR="${configDir.replace(/\\/g, "/")}"\nexport WIPE_CEILING_DIRS="${box.replace(/\\/g, "/")}"\n` +
      `echo "wipe -> ${bin.replace(/\\/g, "/")}"\n`
  );
}

function clone(name, user) {
  const dir = join(scenario, name);
  git(["clone", "-q", join(scenario, "remote.git"), dir], scenario);
  git(["config", "user.name", user], dir);
  git(["config", "user.email", `${name}@example.com`], dir);
  return dir;
}

/** A bare remote and two clones sharing one board on the wipe branch. */
function createScenario() {
  if (existsSync(scenario)) {
    console.log(`scenario kept: ${scenario} (reset recreates it)`);
    return;
  }
  console.log("Creating the scenario…");
  mkdirSync(scenario, { recursive: true });
  git(["init", "-q", "--bare", "-b", "main", "remote.git"], scenario);
  const alice = clone("alice", "Alice");
  writeFileSync(join(alice, "README.md"), "# sandbox project\n");
  git(["add", "."], alice);
  git(["commit", "-q", "-m", "init"], alice);
  wipe(["init", "--name", "Sandbox", "--yes"], alice, "alice");
  git(["add", ".gitignore"], alice);
  git(["commit", "-q", "-m", "ignore the board's working copy"], alice);
  git(["push", "-q", "origin", "main"], alice);
  for (const [title, list] of [
    ["Welcome to the sandbox", "todo"],
    ["Try a conflicting edit from bob", "todo"],
    ["Ship it", "backlog"],
  ]) {
    wipe(["ticket", "create", title, "--list", list], alice, "alice");
  }
  wipe(["commit", "-m", "seed the sandbox board"], alice, "alice");
  const bob = clone("bob", "Bob");
  // The board is checked out of the wipe branch on bob's first command.
  wipe(["status"], bob, "bob");
  console.log(`  ${alice}\n  ${bob}`);
}

function globalSupportsDev() {
  try {
    execSync("wipe dev list", { cwd: root, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function status() {
  if (!existsSync(bin)) {
    console.log("no sandbox yet - node scripts/sandbox.mjs setup");
    return;
  }
  console.log(`sandbox  ${box}`);
  console.log(`build    ${wipe(["--version"], box).trim()}  (${bin})`);
  console.log(`scenario ${existsSync(scenario) ? join(scenario, "{alice,bob}") : "none - reset creates it"}`);
  const env = { ...process.env };
  delete env.WIPE_CONFIG_DIR;
  console.log(run(bin, ["dev", "list"], { env }).trim());
  if (!globalSupportsDev()) {
    console.log(
      "\nThe installed wipe predates `wipe dev` (1.0), so it won't hand off to the sandbox yet.\n" +
        "Until then, activate the sandbox per shell:\n" +
        "  PowerShell:  . tests/sandbox/activate.ps1\n" +
        "  bash/zsh:    source tests/sandbox/activate.sh"
    );
  }
}

switch (cmd) {
  case "setup":
    build(flags.includes("--ui"));
    install();
    seedConfig();
    pin();
    activateScripts();
    createScenario();
    console.log("");
    status();
    console.log(
      "\nTry it:  cd tests/sandbox/scenario/bob ; $env:WIPE_AGENT='bob' ; wipe sync --check\n" +
        "Board:   wipe serve --port 7900   (from a scenario clone)"
    );
    break;
  case "update":
    build(flags.includes("--ui"));
    install();
    break;
  case "reset":
    // A board server or a background fetch may hold files for a moment.
    rmSync(scenario, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
    createScenario();
    break;
  case "status":
    status();
    break;
  case "remove": {
    if (existsSync(bin)) {
      const env = { ...process.env };
      delete env.WIPE_CONFIG_DIR;
      try {
        console.log(run(bin, ["dev", "unlink", box], { env }).trim());
      } catch {
        /* not pinned */
      }
    }
    // Only what this script made: tests/sandbox/ may hold other local boards.
    for (const p of [binDir, configDir, scenario, join(box, "activate.ps1"), join(box, "activate.sh")]) {
      rmSync(p, { recursive: true, force: true });
    }
    console.log(`removed the sandbox build, config and scenario from ${box}`);
    break;
  }
  default:
    console.error("usage: node scripts/sandbox.mjs setup|update [--ui]|reset|status|remove");
    process.exit(2);
}
