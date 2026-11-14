// Copies musterd, muster-hook and the muster CLI into src-tauri/binaries/ with the target-triple
// suffix Tauri's `externalBin` expects. The bundle then carries them in
// Muster.app/Contents/MacOS/, where the shell looks for the daemon first.
//
//   node scripts/prepare-sidecars.mjs [--profile release|debug]
//   MUSTER_BIN_DIR=/path/to/bins node scripts/prepare-sidecars.mjs

import { execFileSync } from "node:child_process";
import { copyFileSync, chmodSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const app = join(here, "..");
const i = process.argv.indexOf("--profile");
const profile = i >= 0 ? process.argv[i + 1] : "release";
const from = process.env.MUSTER_BIN_DIR || join(app, "..", "target", profile);
const triple = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)[1];
const out = join(app, "src-tauri", "binaries");
mkdirSync(out, { recursive: true });

let missing = 0;
// musterd finds muster-hook and the muster CLI next to itself.
for (const name of ["musterd", "muster-hook", "muster"]) {
  const src = join(from, name);
  if (!existsSync(src)) {
    console.error(`missing ${src} (build it with: cargo build --${profile === "release" ? "release" : ""} -p ${name === "muster" ? "muster-cli" : name})`);
    missing++;
    continue;
  }
  const dst = join(out, `${name}-${triple}`);
  copyFileSync(src, dst);
  chmodSync(dst, 0o755);
  console.log(`${src} -> ${dst}`);
}
process.exit(missing ? 1 : 0);
