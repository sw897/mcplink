import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const debug = process.argv.includes("--debug");
const fixtures = process.argv.includes("--test-fixtures");
const profile = debug ? "debug" : "release";
execFileSync(
  "cargo",
  ["build", "--locked", ...(debug ? [] : ["--release"]), "-p", "mcplink-core",
    ...(fixtures ? ["--features", "test-fixtures"] : [])],
  {
    stdio: "inherit",
  },
);
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(
  /^host: (.+)$/m,
)?.[1];
if (!host) throw new Error("Unable to determine Rust target");
mkdirSync("src-tauri/binaries", { recursive: true });
const ext = process.platform === "win32" ? ".exe" : "";
copyFileSync(
  join(process.env.CARGO_TARGET_DIR || "target", profile, `mcplink${ext}`),
  `src-tauri/binaries/mcplink-${host}${ext}`,
);
