import { transformAsync } from "@babel/core";
import * as esbuild from "esbuild";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

const hostExternals = [
  "@impasto/clipboard/host",
  "@impasto/linking/host",
  "@impasto/app-state/host",
  "@impasto/file-system/host",
  "@impasto/async-storage/host",
  "@impasto/permissions/host",
];

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
  nodePaths: [join(here, "node_modules")],
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
