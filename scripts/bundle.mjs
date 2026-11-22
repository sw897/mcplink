import { execFileSync } from "node:child_process";
import { readFileSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import { resolve } from "node:path";

const require = createRequire(import.meta.url);
const env = { ...process.env };
const args = process.argv.slice(2);
const localSigning = args.includes("--local-signing");
const fixtures = args.includes("--test-fixtures");
if (args.some((arg) => arg !== "--test-fixtures" && arg.includes("test-fixtures")))
  throw new Error("Use --test-fixtures to build a separate acceptance package");
if (fixtures && (localSigning || env.MCPLINK_WINDOWS_SIGNING === "1"))
  throw new Error("Acceptance packages cannot use release signing");
if (fixtures) {
  env.CARGO_TARGET_DIR = resolve("target/fixture-bundle");
  for (const key of ["APPLE_SIGNING_IDENTITY", "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_KEY", "APPLE_API_ISSUER", "APPLE_API_KEY_PATH"])
    delete env[key];
}
const target = resolve(env.CARGO_TARGET_DIR || "target");
if (localSigning && process.platform !== "darwin") {
  throw new Error("--local-signing is only available on macOS");
}
if (process.platform === "darwin") {
  if (localSigning) {
    const config = JSON.parse(
      readFileSync(".local/macos-signing.json", "utf8"),
    );
    if (!/^[0-9a-f]{40}$/i.test(config.identity)) {
      throw new Error(
        "Invalid local signing identity; run scripts/setup_macos_signing.py",
      );
    }
    if (
      env.APPLE_SIGNING_IDENTITY &&
      env.APPLE_SIGNING_IDENTITY !== config.identity
    ) {
      throw new Error(
        "Local signing identity conflicts with APPLE_SIGNING_IDENTITY",
      );
    }
    env.APPLE_SIGNING_IDENTITY = config.identity;
  }
  // Preview builds never silently select a developer's installed identity.
  env.APPLE_SIGNING_IDENTITY ||= "-";
}
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

// Cargo caches may retain installers from earlier versions. Publish only this build.
rmSync(resolve(target, "release/bundle"), { recursive: true, force: true });
execFileSync(process.execPath, ["scripts/prepare-sidecar.mjs", ...(fixtures ? ["--test-fixtures"] : [])], {
  stdio: "inherit",
  env,
});
if (process.platform === "darwin") {
  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(
    /^host: (.+)$/m,
  )?.[1];
  if (!host) throw new Error("Unable to determine signing target");
  for (const file of [
    resolve(target, "release/mcplink"),
    `src-tauri/binaries/mcplink-${host}`,
  ]) {
    execFileSync(
      "/usr/bin/codesign",
      [
        "--force",
        "--sign",
        env.APPLE_SIGNING_IDENTITY,
        "--identifier",
        "io.mcplink.cli",
        "--options",
        "runtime",
        ...(localSigning || env.APPLE_SIGNING_IDENTITY === "-"
          ? ["--timestamp=none"]
          : []),
        file,
      ],
      { stdio: "inherit", env },
    );
  }
}
if (process.platform === "win32" && env.MCPLINK_WINDOWS_SIGNING === "1") {
  const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(
    /^host: (.+)$/m,
  )?.[1];
  if (!host) throw new Error("Unable to determine signing target");
  for (const file of [
    resolve(target, "release/mcplink.exe"),
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
    ...(fixtures ? ["--features", "test-fixtures"] : []),
    ...args.filter((arg) => arg !== "--local-signing" && arg !== "--test-fixtures"),
  ],
  { stdio: "inherit", env },
);
if (process.platform === "darwin") {
  for (const file of [
    resolve(target, "release/mcplink"),
    resolve(target, "release/bundle/macos/mcplink.app/Contents/MacOS/mcplink"),
  ]) {
    execFileSync(
      "/usr/bin/codesign",
      ["--verify", "--strict", "-R", '=identifier "io.mcplink.cli"', file],
      { stdio: "inherit" },
    );
  }
}
