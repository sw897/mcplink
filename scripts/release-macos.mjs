import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const signed = process.argv.includes("--signed");
if (process.argv.slice(2).some((arg) => arg !== "--signed")) {
  throw new Error("Usage: npm run release:macos -- [--signed]");
}
if (process.platform !== "darwin" || process.arch !== "arm64") {
  throw new Error("The release entry point currently supports macOS Apple Silicon");
}
if (signed) {
  const env = process.env;
  if (!env.APPLE_SIGNING_IDENTITY?.startsWith("Developer ID Application:")) {
    throw new Error("Set APPLE_SIGNING_IDENTITY to a Developer ID Application identity");
  }
  const api = env.APPLE_API_KEY && env.APPLE_API_ISSUER && env.APPLE_API_KEY_PATH && existsSync(env.APPLE_API_KEY_PATH);
  const appleId = env.APPLE_ID && env.APPLE_PASSWORD && env.APPLE_TEAM_ID;
  if (!api && !appleId) throw new Error("Supply complete Apple notarization credentials through the environment");
  if (!env.APPLE_CERTIFICATE) {
    const identities = execFileSync("security", ["find-identity", "-v", "-p", "codesigning"], { encoding: "utf8" });
    if (!identities.includes(`"${env.APPLE_SIGNING_IDENTITY}"`)) {
      throw new Error("The requested Developer ID certificate is not installed");
    }
  } else if (env.APPLE_CERTIFICATE_PASSWORD === undefined) {
    throw new Error("APPLE_CERTIFICATE_PASSWORD is required with APPLE_CERTIFICATE");
  }
}

const version = JSON.parse(readFileSync("package.json", "utf8")).version;
const stamp = new Date().toISOString().replaceAll(/[:.]/g, "-");
const record = resolve(`artifacts/e2e/release/${stamp}`);
mkdirSync(record, { recursive: true });
const commands = [];
const run = (program, args, env = process.env) => {
  try {
    execFileSync(program, args, { stdio: "inherit", env });
    commands.push({ program, args, exit_status: 0 });
  } catch (error) {
    commands.push({ program, args, exit_status: error.status ?? 1 });
    throw error;
  } finally {
    writeFileSync(join(record, "commands.json"), JSON.stringify(commands, null, 2) + "\n");
  }
};
run("python3", ["scripts/release_manifest.py", "--record", join(record, "source")]);
run("npm", ["run", "check"]);
const bundleEnv = { ...process.env };
if (!signed) {
  bundleEnv.APPLE_SIGNING_IDENTITY = "-";
  for (const key of ["APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_API_KEY", "APPLE_API_ISSUER", "APPLE_API_KEY_PATH"]) delete bundleEnv[key];
}
run("npm", ["run", "bundle"], bundleEnv);
const dmg = resolve(`target/release/bundle/dmg/mcplink_${version}_aarch64.dmg`);
const app = resolve("target/release/bundle/macos/mcplink.app");
run("python3", ["tests/e2e/verify_package.py", "--record", join(record, "package"), ...(signed ? ["--require-notarization"] : [])]);
run("python3", ["tests/e2e/installed_app.py", "--dmg", dmg, "--record", join(record, "installed")]);
run("python3", ["tests/e2e/agent_cli.py", "--record", join(record, "agent-cli")], { ...process.env, MCPLINK_E2E_BINARY: join(app, "Contents/MacOS/mcplink") });
run("python3", ["tests/e2e/daemon_lifecycle.py", "--record", join(record, "lifecycle")], { ...process.env, MCPLINK_E2E_BINARY: join(app, "Contents/MacOS/mcplink") });
run("python3", ["scripts/release_manifest.py", "--record", join(record, "source-after")]);
const sourceBefore = JSON.parse(readFileSync(join(record, "source/release-manifest.json"), "utf8"));
const sourceAfter = JSON.parse(readFileSync(join(record, "source-after/release-manifest.json"), "utf8"));
if (JSON.stringify(sourceBefore.source_files) !== JSON.stringify(sourceAfter.source_files)) {
  throw new Error("Source changed during release validation; rebuild from a stable checkout");
}
const output = resolve(`artifacts/releases/${version}/${stamp}`);
mkdirSync(output, { recursive: true });
const dmgCopy = join(output, `mcplink_${version}_aarch64.dmg`);
copyFileSync(dmg, dmgCopy);
const zip = join(output, `mcplink_${version}_aarch64.app.zip`);
const cli = join(output, `mcplink_cli_${version}_aarch64-apple-darwin.tar.gz`);
run("python3", ["scripts/package_release.py", "--app", app, "--zip", zip, "--cli", cli]);
run("python3", ["tests/e2e/verify_archives.py", "--zip", zip, "--cli", cli, "--record", join(record, "archives"), ...(signed ? ["--require-notarization"] : [])]);
copyFileSync("docs/release-notes.md", join(output, "RELEASE_NOTES.md"));
run("python3", ["scripts/release_manifest.py", "--record", output, "--artifact", dmgCopy, "--artifact", zip, "--artifact", cli]);
const packageReport = JSON.parse(readFileSync(join(record, "package/package-report.json"), "utf8"));
const installedReport = JSON.parse(readFileSync(join(record, "installed/report.json"), "utf8"));
const lifecycleReport = JSON.parse(readFileSync(join(record, "lifecycle/report.json"), "utf8"));
writeFileSync(join(output, "verification.json"), JSON.stringify({ version, signed, developer_id_signed: packageReport.developer_id_signed, notarized: packageReport.notarized, package: packageReport.conclusion, installed: installedReport.conclusion, installed_scenarios: installedReport.scenarios, agent_cli: JSON.parse(readFileSync(join(record, "agent-cli/report.json"), "utf8")).conclusion, lifecycle: lifecycleReport.conclusion, lifecycle_scenarios: lifecycleReport.scenarios, limits: installedReport.limits }, null, 2) + "\n");
console.log(`Release candidate: ${output}`);
