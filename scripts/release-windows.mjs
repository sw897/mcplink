import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";

const signed = process.argv.includes("--signed");
if (process.argv.slice(2).some((arg) => arg !== "--signed"))
  throw new Error("Usage: npm run release:windows -- [--signed]");
if (process.platform !== "win32" || process.arch !== "x64")
  throw new Error("Windows x64 is required");
const env = { ...process.env };
const signScript = resolve("scripts/sign-windows.ps1");
const sign = (args) =>
  execFileSync(
    "powershell.exe",
    ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", signScript, ...args],
    { stdio: "inherit", env },
  );
if (signed) sign(["-PreflightOnly"]);
delete env.MCPLINK_WINDOWS_SIGNING;
if (signed) env.MCPLINK_WINDOWS_SIGNING = "1";
const version = JSON.parse(readFileSync("package.json", "utf8")).version;
const stamp = new Date().toISOString().replaceAll(/[:.]/g, "-");
const record = resolve(`artifacts/e2e/release-windows/${stamp}`);
mkdirSync(record, { recursive: true });
const run = (command, args) =>
  execFileSync(command, args, { stdio: "inherit", env });
if (!env.npm_execpath)
  throw new Error("Start this script through npm run release:windows");
const npm = (args) => run(process.execPath, [env.npm_execpath, ...args]);
run("python", [
  "scripts/release_manifest.py",
  "--require-clean",
  "--record",
  join(record, "source"),
]);
npm(["run", "check"]);
const config = join(record, "signing-config.json");
writeFileSync(
  config,
  JSON.stringify({
    bundle: {
      windows: signed
        ? {
            signCommand: {
              cmd: "powershell.exe",
              args: [
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                signScript,
                "-FilePath",
                "%1",
              ],
            },
          }
        : { signCommand: null, certificateThumbprint: null },
    },
  }),
);
npm(["run", "bundle", "--", "--config", config]);
const installers = readdirSync("target/release/bundle/nsis").filter(
  (n) => n.endsWith("-setup.exe") && n.includes(`_${version}_`),
);
if (installers.length !== 1)
  throw new Error("Expected exactly one version-matching NSIS installer");
const installer = resolve("target/release/bundle/nsis", installers[0]);
if (signed)
  for (const file of [
    installer,
    "target/release/mcplink.exe",
    "target/release/mcplink-desktop.exe",
  ])
    sign(["-VerifyOnly", "-FilePath", resolve(file)]);
env.MCPLINK_E2E_BINARY = resolve("target/release/mcplink.exe");
run("python", [
  "tests/e2e/windows_installer.py",
  "--installer",
  installer,
  "--record",
  join(record, "installed"),
  ...(env.MCPLINK_PREVIOUS_INSTALLER
    ? ["--previous-installer", env.MCPLINK_PREVIOUS_INSTALLER]
    : []),
  ...(signed ? ["--require-signature"] : []),
]);
env.MCPLINK_E2E_BINARY = resolve("target/release/mcplink.exe");
for (const test of [
  "agent_cli",
  "aggregate_gateway",
  "entry_policy",
  "catalog_refresh",
  "cli_workflow",
  "daemon_lifecycle",
])
  run("python", [`tests/e2e/${test}.py`, "--record", join(record, test)]);
run("python", [
  "scripts/release_manifest.py",
  "--require-clean",
  "--record",
  join(record, "source-after"),
  "--binary",
  "target/release/mcplink.exe",
  "--binary",
  "target/release/mcplink-desktop.exe",
]);
const before = JSON.parse(
  readFileSync(join(record, "source/release-manifest.json"), "utf8"),
);
const after = JSON.parse(
  readFileSync(join(record, "source-after/release-manifest.json"), "utf8"),
);
if (JSON.stringify(before.source_files) !== JSON.stringify(after.source_files))
  throw new Error("Source changed during release validation");
const output = resolve(`artifacts/releases/${version}/${stamp}`);
mkdirSync(output, { recursive: true });
const setup = join(output, installers[0]);
copyFileSync(installer, setup);
const cli = join(output, `mcplink_cli_${version}_x86_64-pc-windows-msvc.zip`);
run("python", [
  "scripts/package_windows.py",
  "--binary",
  "target/release/mcplink.exe",
  "--output",
  cli,
]);
copyFileSync("docs/release-notes.md", join(output, "RELEASE_NOTES.md"));
run("python", [
  "scripts/release_manifest.py",
  "--require-clean",
  "--record",
  output,
  "--artifact",
  setup,
  "--artifact",
  cli,
]);
writeFileSync(
  join(output, "verification.json"),
  JSON.stringify(
    {
      version,
      signed,
      publisherSignatureVerified: signed,
      installed: JSON.parse(
        readFileSync(join(record, "installed/report.json"), "utf8"),
      ),
      suites: Object.fromEntries(
        [
          "agent_cli",
          "aggregate_gateway",
          "entry_policy",
          "catalog_refresh",
          "cli_workflow",
          "daemon_lifecycle",
        ].map((test) => [
          test,
          JSON.parse(readFileSync(join(record, test, "report.json"), "utf8"))
            .conclusion,
        ]),
      ),
    },
    null,
    2,
  ) + "\n",
);
console.log(`Release candidate: ${output}`);
