import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { resolve } from "node:path";

const require = createRequire(import.meta.url);
const env = { ...process.env };
const flags =
  env.CARGO_ENCODED_RUSTFLAGS !== undefined
    ? env.CARGO_ENCODED_RUSTFLAGS.split("\x1f")
    : (env.RUSTFLAGS || "").split(/\s+/).filter(Boolean);
// Map both dependency and workspace source locations out of distributed binaries.
flags.push(`--remap-path-prefix=${homedir()}=/user-home`);
flags.push(`--remap-path-prefix=${process.cwd()}=.`);
env.CARGO_ENCODED_RUSTFLAGS = flags.join("\x1f");

if (process.platform === "win32") {
  // MSVC's native dependencies embed __FILE__ independently of Rust's remap.
  // cc tracks _CL_ for rebuilds; preserve the caller's options and quote one arg.
  const trim = `/d1trimfile:${homedir()}`
    .replace(/(\\*)"/g, '$1$1\\"')
    .replace(/(\\+)$/, "$1$1");
  env._CL_ = [process.env._CL_, `"${trim}"`].filter(Boolean).join(" ");
}

execFileSync(process.execPath, ["scripts/prepare-sidecar.mjs"], {
  stdio: "inherit",
  env,
});
if (process.platform === "win32" && env.MCPLINK_WINDOWS_SIGNING === "1") {
  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(
    /^host: (.+)$/m,
  )?.[1];
  if (!host) throw new Error("Unable to determine signing target");
  for (const file of [
    "target/release/mcplink.exe",
    `src-tauri/binaries/mcplink-${host}.exe`,
  ]) {
    execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        resolve("scripts/sign-windows.ps1"),
        "-FilePath",
        resolve(file),
      ],
      { stdio: "inherit", env },
    );
  }
}
execFileSync(
  process.execPath,
  [
    require.resolve("@tauri-apps/cli/tauri.js"),
    "build",
    ...process.argv.slice(2),
  ],
  { stdio: "inherit", env },
);
