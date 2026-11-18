import * as esbuild from "esbuild";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    dev: { type: "boolean", default: false },
    port: { type: "string", default: "1420" },
  },
});
const dev = values.dev;
const port = Number(values.port);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error("Port must be an integer from 1 to 65535");
}

const root = process.cwd();
const dist = resolve(root, "dist");
const entry = resolve(root, "src/main.tsx");
const template = resolve(root, "index.html");
const entryTag = '<script type="module" src="/src/main.tsx"></script>';
const assetUrl = (file) =>
  "/" + relative(dist, resolve(root, file)).split(sep).join("/");

// Serve only generated output, never the project directory or local configuration.
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
if (existsSync(resolve(root, "public"))) {
  cpSync(resolve(root, "public"), dist, { recursive: true });
}

const options = {
  absWorkingDir: root,
  entryPoints: [{ in: entry, out: "index" }],
  outdir: dist,
  entryNames: dev ? "assets/[name]" : "assets/[name]-[hash]",
  assetNames: "assets/[name]-[hash]",
  bundle: true,
  format: "esm",
  platform: "browser",
  target: ["chrome107", "edge107", "firefox104", "safari16"],
  jsx: "automatic",
  jsxDev: dev,
  minify: !dev,
  sourcemap: dev ? "inline" : false,
  metafile: true,
  legalComments: "eof",
  define: {
    "process.env.NODE_ENV": JSON.stringify(dev ? "development" : "production"),
  },
  loader: {
    ".woff2": "file",
    ".woff": "file",
    ".ttf": "file",
    ".png": "file",
    ".svg": "file",
  },
  ...(dev
    ? {
        banner: {
          js: 'new EventSource("/esbuild").addEventListener("change", () => location.reload());',
        },
      }
    : {}),
  plugins: [
    {
      name: "html-entry",
      setup(build) {
        build.onLoad({ filter: /[\\/]src[\\/]main\.tsx$/ }, (args) => {
          // HTML is written separately from esbuild's outputs. Preserve a dev
          // revision comment so template-only edits also emit a reload event.
          const revision = dev
            ? `/*! @preserve mcplink HTML revision ${createHash("sha256").update(readFileSync(template)).digest("hex")} */\n`
            : "";
          return {
            contents: revision + readFileSync(args.path, "utf8"),
            loader: "tsx",
            resolveDir: dirname(args.path),
            watchFiles: [template],
          };
        });
        build.onEnd((result) => {
          if (result.errors.length) return;
          const output = Object.entries(result.metafile.outputs).find(
            ([file, data]) =>
              file.endsWith(".js") &&
              data.entryPoint &&
              resolve(root, data.entryPoint) === entry,
          );
          if (!output) throw new Error("Missing frontend entry output");
          let html = readFileSync(template, "utf8");
          if (!html.includes(entryTag) || !html.includes("</head>")) {
            throw new Error(
              "index.html must contain the frontend module entry and a head element",
            );
          }
          html = html.replace(
            entryTag,
            `<script type="module" src="${assetUrl(output[0])}"></script>`,
          );
          if (output[1].cssBundle) {
            html = html.replace(
              "</head>",
              `<link rel="stylesheet" href="${assetUrl(output[1].cssBundle)}"/></head>`,
            );
          }
          writeFileSync(resolve(dist, "index.html"), html);
        });
      },
    },
  ],
};

if (dev) {
  const context = await esbuild.context(options);
  try {
    await context.rebuild();
    await context.watch();
    const server = await context.serve({
      host: "127.0.0.1",
      port,
      servedir: dist,
    });
    if (server.port !== port)
      throw new Error("Requested development port is unavailable");
    for (const signal of ["SIGINT", "SIGTERM"]) {
      process.once(
        signal,
        () => void context.dispose().then(() => process.exit(0)),
      );
    }
    console.log(`mcplink frontend: http://127.0.0.1:${port}`);
  } catch (error) {
    await context.dispose();
    throw error;
  }
} else {
  await esbuild.build(options);
  esbuild.stop();
}
