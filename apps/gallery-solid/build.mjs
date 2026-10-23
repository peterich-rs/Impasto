import { transformAsync } from "@babel/core";
import * as esbuild from "esbuild";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));

const solid = {
  name: "solid-universal",
  setup(build) {
    build.onLoad({ filter: /\.tsx?$/ }, async (args) => {
      if (args.path.endsWith(".d.ts")) {
        return;
      }
      const source = readFileSync(args.path, "utf8");
      const result = await transformAsync(source, {
        filename: args.path,
        babelrc: false,
        configFile: false,
        sourceMaps: false,
        plugins: [
          ["@babel/plugin-transform-typescript", { isTSX: true, allExtensions: true }],
          [
            "@solidjs/babel-plugin",
            {
              moduleName: "@uifw/solid-renderer",
              generate: "universal",
            },
          ],
        ],
      });
      if (result?.code == null) {
        throw new Error(`babel produced no code for ${args.path}`);
      }
      return { contents: result.code, loader: "js" };
    });
  },
};

await esbuild.build({
  absWorkingDir: here,
  entryPoints: ["counter.tsx"],
  outfile: "dist/counter.js",
  bundle: true,
  format: "iife",
  platform: "browser",
  conditions: ["browser"],
  target: "es2022",
  plugins: [solid],
  nodePaths: [join(here, "node_modules")],
  define: {
    "process.env.NODE_ENV": '"production"',
  },
  logLevel: "info",
});
