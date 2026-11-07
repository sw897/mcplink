import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { homedir } from "node:os";

const require = createRequire(import.meta.url);
const env = { ...process.env };
const flags = env.CARGO_ENCODED_RUSTFLAGS !== undefined
  ? env.CARGO_ENCODED_RUSTFLAGS.split("\x1f")
  : (env.RUSTFLAGS || "").split(/\s+/).filter(Boolean);
// Map both dependency and workspace source locations out of distributed binaries.
flags.push(`--remap-path-prefix=${homedir()}=/user-home`);
flags.push(`--remap-path-prefix=${process.cwd()}=.`);
env.CARGO_ENCODED_RUSTFLAGS = flags.join("\x1f");

execFileSync(process.execPath, ["scripts/prepare-sidecar.mjs"], { stdio: "inherit", env });
execFileSync(process.execPath, [require.resolve("@tauri-apps/cli/tauri.js"), "build", ...process.argv.slice(2)], { stdio: "inherit", env });
