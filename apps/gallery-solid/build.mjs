import { transformAsync } from "@babel/core";
import * as esbuild from "esbuild";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

// Host 桥包由各包 package.json 的 `./host` export 声明，不在清单里枚举。
const hostExternals = [];
const packagesRoot = join(here, "..", "..", "packages");
for (const entry of readdirSync(packagesRoot, { withFileTypes: true })) {
  if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
  const manifest = JSON.parse(
    readFileSync(join(packagesRoot, entry.name, "package.json"), "utf8"),
  );
  if (manifest.exports?.["./host"]) {
    hostExternals.push(`@impasto/${entry.name}/host`);
  }
}

const solid = {
  name: "solid-universal",
  setup(build) {
    build.onLoad({ filter: /\.tsx?$/ }, async (args) => {
      if (args.path.endsWith(".d.ts") || args.path.endsWith(".gen.ts")) {
        return;
      }
      const source = readFileSync(args.path, "utf8");
      const result = await transformAsync(source, {
        filename: args.path,
        babelrc: false,
        configFile: false,
        sourceMaps: true,
        plugins: [
          ["@babel/plugin-transform-typescript", { isTSX: true, allExtensions: true }],
          [
            "@solidjs/babel-plugin",
            {
              moduleName: "@impasto/solid-renderer",
              generate: "universal",
            },
          ],
        ],
      });
      if (result?.code == null) {
        throw new Error(`babel produced no code for ${args.path}`);
      }
      const contents = result.code.replace(
        /import\(\s*(["'])@impasto\/linking\1\s*\)/g,
        'import("@impasto/linking/host").then((host) => ({ Linking: host }))',
      );
      return { contents, loader: "js" };
    });
  },
};

const options = {
  absWorkingDir: here,
  entryPoints: ["counter.tsx"],
  outfile: "dist/counter.js",
  bundle: true,
  format: "esm",
  platform: "browser",
  conditions: ["browser"],
  target: "es2022",
  plugins: [solid],
  sourcemap: true,
  external: hostExternals,
  define: {
    "process.env.NODE_ENV": '"production"',
  },
  logLevel: "info",
};

if (process.argv.includes("--watch")) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
} else {
  await esbuild.build(options);
}
