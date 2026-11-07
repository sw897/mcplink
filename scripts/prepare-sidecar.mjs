import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";

const debug = process.argv.includes("--debug");
const profile = debug ? "debug" : "release";
execFileSync(
  "cargo",
  ["build", "--locked", ...(debug ? [] : ["--release"]), "-p", "mcplink-core"],
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
  `target/${profile}/mcplink${ext}`,
  `src-tauri/binaries/mcplink-${host}${ext}`,
);
