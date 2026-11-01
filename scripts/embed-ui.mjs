// Rebuild the desktop UI and stage it as the daemon's embedded assets
// (crates/wipe-daemon/assets/, baked into the wipe binary). Shared by
// set-version.mjs (every release) and sandbox.mjs (`update --ui`).
import { existsSync, readdirSync, rmSync, cpSync } from "node:fs";
import { execSync } from "node:child_process";
import { join } from "node:path";

export function embedUi(root) {
  const ui = join(root, "apps/desktop");
  if (!existsSync(join(ui, "package.json"))) return;
  const assets = join(root, "crates/wipe-daemon/assets");
  console.log("Rebuilding + embedding the desktop UI…");
  // Always install: a bumped pnpm-lock.yaml (deps changed since last build) would
  // otherwise build against stale node_modules. --frozen-lockfile is fast when
  // already up to date and fails loudly if the lockfile is out of sync.
  execSync("pnpm install --frozen-lockfile", { cwd: ui, stdio: "inherit" });
  execSync("pnpm build", { cwd: ui, stdio: "inherit" });
  const build = join(ui, "build");
  if (!existsSync(build)) throw new Error(`expected SvelteKit output at ${build}`);
  // Clear the staged assets (keep .gitkeep) and copy the fresh build in.
  for (const entry of readdirSync(assets)) {
    if (entry === ".gitkeep") continue;
    rmSync(join(assets, entry), { recursive: true, force: true });
  }
  cpSync(build, assets, { recursive: true });
  console.log("  embedded fresh UI into crates/wipe-daemon/assets\n");
}
